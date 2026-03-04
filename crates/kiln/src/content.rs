use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use kiln_doc::ast::Metadata;
use regex::Regex;

// ── Content structs ──

#[derive(Debug, Clone)]
pub struct ContentItem {
    pub meta: Metadata,
    pub path: String,
    pub slug: String,
    pub md_path: PathBuf,
    pub source_dir: PathBuf,
}

// ── Content tree ──

#[derive(Debug)]
pub enum ContentKind {
    Page(ContentItem),
    Post(ContentItem),
    /// A directory with no index.md — groups children but has no page.
    Virtual {
        slug: String,
    },
}

/// Section-level config extracted from index.md frontmatter `extra` keys.
#[derive(Debug, Clone, Default)]
pub struct SectionConfig {
    pub slug_pattern: Option<String>,
}

#[derive(Debug)]
pub struct ContentNode {
    pub path: String,
    pub slug: String,
    pub kind: ContentKind,
    pub children: Vec<ContentNode>,
    pub position: usize,
}

/// Result of content discovery: a tree plus flattened convenience lists.
#[derive(Debug)]
pub struct ContentTree {
    pub root: ContentNode,
    pub posts: Vec<ContentItem>,
    pub pages: Vec<ContentItem>,
}

impl ContentTree {
    fn from_root(root: ContentNode) -> Self {
        let mut posts = Vec::new();
        let mut pages = Vec::new();
        collect_flat(&root, &mut posts, &mut pages);
        posts.sort_by(|a, b| {
            let da = a.meta.date.as_deref().unwrap_or("");
            let db = b.meta.date.as_deref().unwrap_or("");
            db.cmp(da)
        });
        pages.sort_by(|a, b| a.path.cmp(&b.path));
        Self { root, posts, pages }
    }
}

fn collect_flat(node: &ContentNode, posts: &mut Vec<ContentItem>, pages: &mut Vec<ContentItem>) {
    match &node.kind {
        ContentKind::Post(p) => posts.push(p.clone()),
        ContentKind::Page(p) => pages.push(p.clone()),
        ContentKind::Virtual { .. } => {}
    }
    for child in &node.children {
        collect_flat(child, posts, pages);
    }
}

// ── Discovery ──

/// Discover all content recursively and return a content tree.
pub fn discover_content(
    content_dir: &Path,
    include_drafts: bool,
    slug_pattern: Option<&Regex>,
) -> anyhow::Result<ContentTree> {
    let children = if content_dir.exists() {
        discover_dir(content_dir, "", include_drafts, slug_pattern, None)?
    } else {
        Vec::new()
    };

    let root = ContentNode {
        path: String::new(),
        slug: String::new(),
        kind: ContentKind::Virtual {
            slug: String::new(),
        },
        children,
        position: 0,
    };

    let tree = ContentTree::from_root(root);

    // Check for path collisions
    let mut seen = HashSet::new();
    check_collisions(&tree.root, &mut seen)?;

    Ok(tree)
}

fn check_collisions(node: &ContentNode, seen: &mut HashSet<String>) -> anyhow::Result<()> {
    if !node.path.is_empty()
        && let ContentKind::Page(_) | ContentKind::Post(_) = &node.kind
        && !seen.insert(node.path.clone())
    {
        anyhow::bail!("duplicate content path: '{}'", node.path);
    }
    for child in &node.children {
        check_collisions(child, seen)?;
    }
    Ok(())
}

fn discover_dir(
    dir: &Path,
    parent_path: &str,
    include_drafts: bool,
    slug_pattern: Option<&Regex>,
    parent_section_config: Option<&SectionConfig>,
) -> anyhow::Result<Vec<ContentNode>> {
    // Determine effective slug pattern: section override > parent pattern
    let section_re;
    let effective_pattern = if let Some(override_pat) =
        parent_section_config.and_then(|sc| sc.slug_pattern.as_deref())
    {
        section_re = Regex::new(override_pat)
            .with_context(|| format!("invalid section_slug_pattern: {override_pat}"))?;
        Some(&section_re)
    } else {
        slug_pattern
    };

    let mut nodes = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            nodes.extend(discover_dir_entry(
                &path,
                parent_path,
                include_drafts,
                effective_pattern,
            )?);
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let file_name = path
            .file_stem()
            .expect("md file has stem")
            .to_string_lossy();
        if file_name == "index" {
            continue; // handled by parent directory
        }
        nodes.extend(discover_file_entry(
            &path,
            parent_path,
            include_drafts,
            effective_pattern,
        )?);
    }

    // Sort by original filename for stable ordering
    nodes.sort_by(|a, b| a.slug.cmp(&b.slug));

    // Assign position indices
    for (i, node) in nodes.iter_mut().enumerate() {
        node.position = i;
    }

    Ok(nodes)
}

fn discover_dir_entry(
    dir: &Path,
    parent_path: &str,
    include_drafts: bool,
    slug_pattern: Option<&Regex>,
) -> anyhow::Result<Option<ContentNode>> {
    let dir_name = dir
        .file_name()
        .expect("dir has name")
        .to_string_lossy()
        .into_owned();
    let index_path = dir.join("index.md");

    if index_path.exists() {
        // Directory with index.md — real content entry + possible section
        let markdown = std::fs::read_to_string(&index_path)
            .with_context(|| format!("failed to read {}", index_path.display()))?;
        let doc = kiln_doc::parse::parse(&markdown)
            .map_err(|e| anyhow::anyhow!("parse error in {}: {e}", index_path.display()))?;

        if doc.meta.draft && !include_drafts {
            return Ok(None); // skip entire subtree
        }

        let slug = derive_slug(&dir_name, &doc.meta, slug_pattern);
        let path = join_path(parent_path, &slug);
        let section_config = extract_section_config(&doc.meta);

        // Recurse into children
        let children = discover_dir(
            dir,
            &path,
            include_drafts,
            slug_pattern,
            section_config.as_ref(),
        )?;

        let item = ContentItem {
            meta: doc.meta,
            path: path.clone(),
            slug: slug.clone(),
            md_path: index_path,
            source_dir: dir.to_path_buf(),
        };
        let kind = if item.meta.date.is_some() {
            ContentKind::Post(item)
        } else {
            ContentKind::Page(item)
        };

        Ok(Some(ContentNode {
            path,
            slug,
            kind,
            children,
            position: 0,
        }))
    } else {
        // Directory without index.md — virtual section, still recurse
        let slug = apply_slug_pattern(&dir_name, slug_pattern);
        let path = join_path(parent_path, &slug);

        let children = discover_dir(dir, &path, include_drafts, slug_pattern, None)?;

        if children.is_empty() {
            return Ok(None); // empty virtual section, skip
        }

        Ok(Some(ContentNode {
            path,
            slug,
            kind: ContentKind::Virtual { slug: dir_name },
            children,
            position: 0,
        }))
    }
}

fn discover_file_entry(
    file: &Path,
    parent_path: &str,
    include_drafts: bool,
    slug_pattern: Option<&Regex>,
) -> anyhow::Result<Option<ContentNode>> {
    let file_stem = file
        .file_stem()
        .expect("md file has stem")
        .to_string_lossy()
        .into_owned();
    let markdown = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let doc = kiln_doc::parse::parse(&markdown)
        .map_err(|e| anyhow::anyhow!("parse error in {}: {e}", file.display()))?;

    if doc.meta.draft && !include_drafts {
        return Ok(None);
    }

    let slug = derive_slug(&file_stem, &doc.meta, slug_pattern);
    let path = join_path(parent_path, &slug);

    let item = ContentItem {
        meta: doc.meta,
        path: path.clone(),
        slug: slug.clone(),
        md_path: file.to_path_buf(),
        source_dir: file.parent().expect("file has parent dir").to_path_buf(),
    };
    let kind = if item.meta.date.is_some() {
        ContentKind::Post(item)
    } else {
        ContentKind::Page(item)
    };

    Ok(Some(ContentNode {
        path,
        slug,
        kind,
        children: Vec::new(),
        position: 0,
    }))
}

// ── Helpers ──

fn join_path(parent: &str, segment: &str) -> String {
    if parent.is_empty() {
        segment.to_string()
    } else {
        format!("{parent}/{segment}")
    }
}

/// Derive slug from a raw filename, applying frontmatter override then pattern.
fn derive_slug(raw_name: &str, meta: &Metadata, slug_pattern: Option<&Regex>) -> String {
    if let Some(ref s) = meta.slug {
        return s.clone();
    }
    apply_slug_pattern(raw_name, slug_pattern)
}

/// Apply slug_pattern regex to a single name segment.
fn apply_slug_pattern(raw_name: &str, slug_pattern: Option<&Regex>) -> String {
    if let Some(re) = slug_pattern
        && let Some(caps) = re.captures(raw_name)
        && let Some(m) = caps.get(1)
    {
        return m.as_str().to_string();
    }
    raw_name.to_string()
}

fn extract_section_config(meta: &Metadata) -> Option<SectionConfig> {
    if let Some(v) = meta.extra.get("section_slug_pattern")
        && let Some(s) = v.as_str()
    {
        return Some(SectionConfig {
            slug_pattern: Some(s.to_string()),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp dir with content files, returning the path.
    fn write_content(dir: &Path, relative: &str, content: &str) {
        let path = dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, content).unwrap();
    }

    #[test]
    fn discover_basic_post_and_page() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(
            &content,
            "my-post/index.md",
            "---\ntitle: Post\ndate: \"2026-01-01\"\n---\nHello",
        );
        write_content(&content, "about/index.md", "---\ntitle: About\n---\nPage");

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.posts.len(), 1);
        assert_eq!(tree.pages.len(), 1);
        assert_eq!(tree.posts[0].slug, "my-post");
        assert_eq!(tree.pages[0].slug, "about");
    }

    #[test]
    fn discover_drafts_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(
            &content,
            "draft-post/index.md",
            "---\ntitle: Draft\ndate: \"2026-01-01\"\ndraft: true\n---\n",
        );
        write_content(
            &content,
            "published/index.md",
            "---\ntitle: Pub\ndate: \"2026-01-02\"\n---\n",
        );

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.posts.len(), 1);
        assert_eq!(tree.posts[0].slug, "published");
    }

    #[test]
    fn discover_drafts_included() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(
            &content,
            "draft-post/index.md",
            "---\ntitle: Draft\ndate: \"2026-01-01\"\ndraft: true\n---\n",
        );
        write_content(
            &content,
            "published/index.md",
            "---\ntitle: Pub\ndate: \"2026-01-02\"\n---\n",
        );

        let tree = discover_content(&content, true, None).unwrap();
        assert_eq!(tree.posts.len(), 2);
    }

    #[test]
    fn discover_slug_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(&content, "01-intro.md", "---\ntitle: Intro\n---\n");
        write_content(&content, "02-setup.md", "---\ntitle: Setup\n---\n");

        let re = Regex::new(r"^\d+-(.+)$").unwrap();
        let tree = discover_content(&content, false, Some(&re)).unwrap();
        assert_eq!(tree.pages.len(), 2);

        let slugs: Vec<&str> = tree.pages.iter().map(|p| p.slug.as_str()).collect();
        assert!(slugs.contains(&"intro"));
        assert!(slugs.contains(&"setup"));
    }

    #[test]
    fn discover_slug_frontmatter_override() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(
            &content,
            "ugly-name/index.md",
            "---\ntitle: Nice\nslug: nice-slug\n---\n",
        );

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.pages.len(), 1);
        assert_eq!(tree.pages[0].slug, "nice-slug");
        assert_eq!(tree.pages[0].path, "nice-slug");
    }

    #[test]
    fn discover_collision_detected() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        // Two entries that resolve to the same slug
        write_content(
            &content,
            "hello/index.md",
            "---\ntitle: A\nslug: same\n---\n",
        );
        write_content(
            &content,
            "world/index.md",
            "---\ntitle: B\nslug: same\n---\n",
        );

        let result = discover_content(&content, false, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn discover_virtual_section() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        // Directory without index.md but with a child
        write_content(&content, "guides/intro.md", "---\ntitle: Intro\n---\n");

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.pages.len(), 1);
        assert_eq!(tree.pages[0].path, "guides/intro");

        // Root should have one virtual child
        assert_eq!(tree.root.children.len(), 1);
        assert!(matches!(
            &tree.root.children[0].kind,
            ContentKind::Virtual { .. }
        ));
    }

    #[test]
    fn discover_empty_dir_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        std::fs::create_dir_all(content.join("empty-dir")).unwrap();

        let tree = discover_content(&content, false, None).unwrap();
        assert!(tree.root.children.is_empty());
        assert!(tree.posts.is_empty());
        assert!(tree.pages.is_empty());
    }

    #[test]
    fn discover_nested_content() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(&content, "docs/index.md", "---\ntitle: Docs\n---\n");
        write_content(&content, "docs/setup/index.md", "---\ntitle: Setup\n---\n");

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.pages.len(), 2);

        let paths: Vec<&str> = tree.pages.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"docs"));
        assert!(paths.contains(&"docs/setup"));
    }

    #[test]
    fn discover_post_sorting() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(
            &content,
            "old/index.md",
            "---\ntitle: Old\ndate: \"2025-01-01\"\n---\n",
        );
        write_content(
            &content,
            "new/index.md",
            "---\ntitle: New\ndate: \"2026-06-01\"\n---\n",
        );
        write_content(
            &content,
            "mid/index.md",
            "---\ntitle: Mid\ndate: \"2025-06-01\"\n---\n",
        );

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.posts.len(), 3);
        // Newest first
        assert_eq!(tree.posts[0].meta.date.as_deref(), Some("2026-06-01"));
        assert_eq!(tree.posts[1].meta.date.as_deref(), Some("2025-06-01"));
        assert_eq!(tree.posts[2].meta.date.as_deref(), Some("2025-01-01"));
    }

    #[test]
    fn discover_page_sorting() {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("content");
        write_content(&content, "zebra/index.md", "---\ntitle: Zebra\n---\n");
        write_content(&content, "alpha/index.md", "---\ntitle: Alpha\n---\n");
        write_content(&content, "middle/index.md", "---\ntitle: Middle\n---\n");

        let tree = discover_content(&content, false, None).unwrap();
        assert_eq!(tree.pages.len(), 3);
        // Alphabetical by path
        assert_eq!(tree.pages[0].path, "alpha");
        assert_eq!(tree.pages[1].path, "middle");
        assert_eq!(tree.pages[2].path, "zebra");
    }
}
