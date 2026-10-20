//! Redirects from an entry's earlier addresses, its front-matter `aliases`, so
//! that links keep working after a rename.

use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use kiln_doc::html::escape_attr;

use crate::content::{ContentItem, ContentTree};

/// Write a redirect page at each alias of each post and page, and return how
/// many were written. An alias that collides with a page, or with another
/// alias, is an error.
pub fn write_aliases(tree: &ContentTree, site_url: &str, output_dir: &Path) -> anyhow::Result<usize> {
    let entries: Vec<(&ContentItem, String)> = tree
        .posts
        .iter()
        .map(|post| (post, format!("/posts/{}/", post.path)))
        .chain(tree.pages.iter().map(|page| (page, format!("/{}/", page.path))))
        .collect();
    let mut taken: HashSet<String> = entries.iter().map(|(_, url)| url.clone()).collect();

    let aliases = entries
        .iter()
        .flat_map(|(item, url)| item.meta.aliases.iter().map(move |alias| (alias, url)));
    let mut written = 0;
    for (alias, url) in aliases {
        let from = alias_url(alias, url)?;
        if !taken.insert(from.clone()) {
            bail!("alias {alias:?} of {url} is already the address of another page or alias");
        }
        let dir = output_dir.join(from.trim_matches('/'));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("index.html"), redirect_html(site_url, url))?;
        written += 1;
    }
    Ok(written)
}

/// The address an alias stands for. Without a leading slash, an alias is an
/// earlier slug: the entry's own address with its last part replaced.
fn alias_url(alias: &str, url: &str) -> anyhow::Result<String> {
    let path = alias.trim_matches('/');
    if path.is_empty() || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        bail!("alias {alias:?} of {url} is not a valid address");
    }
    if alias.starts_with('/') {
        return Ok(format!("/{path}/"));
    }
    let parent = url.trim_end_matches('/').rsplit_once('/').map_or("", |(parent, _)| parent);
    Ok(format!("{parent}/{path}/"))
}

/// A page that sends the browser on to `url`, keeping any `#fragment`, and
/// tells search engines the new address is the one to index.
fn redirect_html(site_url: &str, url: &str) -> String {
    let href = escape_attr(url);
    let canonical = escape_attr(&format!("{}{url}", site_url.trim_end_matches('/')));
    // As a JavaScript string; `<` is escaped so that the string can't end the script.
    let target = serde_json::to_string(url).unwrap_or_default().replace('<', "\\u003c");
    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <title>Moved</title>\n\
         <link rel=\"canonical\" href=\"{canonical}\">\n\
         <meta name=\"robots\" content=\"noindex\">\n\
         <meta http-equiv=\"refresh\" content=\"0; url={href}\">\n\
         <script>location.replace({target} + location.hash)</script>\n\
         </head>\n\
         <body>\n\
         <p>This page has moved to <a href=\"{href}\">{href}</a>.</p>\n\
         </body>\n\
         </html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::discover_content;

    fn site(files: &[(&str, &str)]) -> (tempfile::TempDir, ContentTree) {
        let tmp = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = tmp.path().join("content").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let tree = discover_content(&tmp.path().join("content"), false, None).unwrap();
        (tmp, tree)
    }

    #[test]
    fn an_earlier_slug_redirects_to_the_post() {
        let post = "---\ntitle: New\ndate: \"2026-01-01\"\naliases: [old-name]\n---\nText.";
        let (tmp, tree) = site(&[("new-name/index.md", post)]);
        let out = tmp.path().join("dist");
        assert_eq!(write_aliases(&tree, "https://example.com/", &out).unwrap(), 1);
        let html = std::fs::read_to_string(out.join("posts/old-name/index.html")).unwrap();
        assert!(html.contains("<meta http-equiv=\"refresh\" content=\"0; url=/posts/new-name/\">"));
        assert!(html.contains("<link rel=\"canonical\" href=\"https://example.com/posts/new-name/\">"));
        assert!(html.contains("location.replace(\"/posts/new-name/\" + location.hash)"));
    }

    #[test]
    fn a_nested_page_alias_stays_in_its_section() {
        let page = "---\ntitle: Setup\naliases: [install, /start]\n---\nText.";
        let (tmp, tree) = site(&[("guides/index.md", "---\ntitle: Guides\n---\n"), ("guides/setup/index.md", page)]);
        let out = tmp.path().join("dist");
        assert_eq!(write_aliases(&tree, "https://example.com", &out).unwrap(), 2);
        assert!(out.join("guides/install/index.html").exists());
        assert!(out.join("start/index.html").exists());
    }

    #[test]
    fn an_alias_cannot_take_another_page() {
        let page = "---\ntitle: B\naliases: [a]\n---\n";
        let (tmp, tree) = site(&[("a/index.md", "---\ntitle: A\n---\n"), ("b/index.md", page)]);
        let err = write_aliases(&tree, "", &tmp.path().join("dist")).unwrap_err();
        assert!(err.to_string().contains("already the address"), "{err}");
    }

    #[test]
    fn an_alias_cannot_leave_the_site() {
        let page = "---\ntitle: B\naliases: [\"../x\"]\n---\n";
        let (tmp, tree) = site(&[("b/index.md", page)]);
        assert!(write_aliases(&tree, "", &tmp.path().join("dist")).is_err());
    }
}
