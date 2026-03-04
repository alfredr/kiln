use std::path::{Path, PathBuf};

use anyhow::Context;

const KILN_TOML: &str = r##"[site]
title = "{{TITLE}}"
url = "http://localhost:3000"
description = ""
author = ""
"##;

const BASE_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{% block title %}{{ site_title }}{% endblock %}</title>
  <link rel="stylesheet" href="/css/main.css">
</head>
<body>
  <nav>
    <a href="/">{{ site_title }}</a>
  </nav>
  <main>
    {% block content %}{% endblock %}
  </main>
  <footer>
    <p>Built with <a href="#">kiln</a></p>
  </footer>
</body>
</html>
"##;

const POST_HTML: &str = r##"{% extends "base.html" %}

{% block title %}{{ title }} — {{ site_title }}{% endblock %}

{% block content %}
<article>
  <header>
    <h1>{{ title }}</h1>
    <time datetime="{{ date }}">{{ date }}</time>
    {% if tags %}
    <div class="tags">
      {% for tag in tags %}<span class="tag">{{ tag }}</span>{% endfor %}
    </div>
    {% endif %}
  </header>
  <div class="post-body">
    {{ body | safe }}
  </div>
</article>
{% endblock %}
"##;

const MAIN_CSS: &str = r##"*,
*::before,
*::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

body {
  font-family: system-ui, -apple-system, sans-serif;
  line-height: 1.6;
  max-width: 48rem;
  margin: 0 auto;
  padding: 2rem 1rem;
  color: #1a1a1a;
  background: #fafafa;
}

nav {
  margin-bottom: 2rem;
  padding-bottom: 1rem;
  border-bottom: 1px solid #ddd;
}

nav a {
  font-weight: 700;
  font-size: 1.25rem;
  text-decoration: none;
  color: #1a1a1a;
}

article header {
  margin-bottom: 2rem;
}

article header h1 {
  font-size: 2rem;
  line-height: 1.2;
  margin-bottom: 0.5rem;
}

article header time {
  color: #666;
  font-size: 0.9rem;
}

.tags {
  margin-top: 0.5rem;
}

.tag {
  display: inline-block;
  background: #e8e8e8;
  padding: 0.15rem 0.5rem;
  border-radius: 3px;
  font-size: 0.8rem;
  margin-right: 0.25rem;
}

.post-body h1,
.post-body h2,
.post-body h3 {
  margin-top: 1.5em;
  margin-bottom: 0.5em;
}

.post-body p {
  margin-bottom: 1em;
}

.post-body ul,
.post-body ol {
  margin-bottom: 1em;
  padding-left: 1.5rem;
}

.post-body li {
  margin-bottom: 0.25em;
}

.post-body code {
  background: #f0f0f0;
  padding: 0.1em 0.3em;
  border-radius: 3px;
  font-size: 0.9em;
}

.post-body pre {
  background: #2b303b;
  color: #c0c5ce;
  padding: 1rem;
  border-radius: 6px;
  overflow-x: auto;
  margin-bottom: 1em;
}

.post-body pre code {
  background: none;
  padding: 0;
}

footer {
  margin-top: 3rem;
  padding-top: 1rem;
  border-top: 1px solid #ddd;
  color: #999;
  font-size: 0.85rem;
}
"##;

const SAMPLE_POST: &str = r##"---
title: Hello from kiln
date: {{DATE}}
tags: [kiln]
summary: First post built with kiln.
---

# Hello from kiln

This is the first post compiled by *kiln*.

## Getting Started

Edit this file at `content/hello/index.md` and run `kiln build` to compile.

## Features

- Markdown-native document model
- Clean, semantic HTML output
- Simple template system
"##;

const PAGE_HTML: &str = r##"{% extends "base.html" %}

{% block title %}{{ title }} — {{ site_title }}{% endblock %}

{% block content %}
<article>
  <h1>{{ title }}</h1>
  <div class="page-body">
    {{ body | safe }}
  </div>
</article>
{% endblock %}
"##;

const SAMPLE_PAGE: &str = r##"---
title: About
---

# About

This is an example page. Pages are standalone content that lives at top-level URLs like `/about/`.

Unlike posts, pages don't have dates or tags — they're for permanent content like "About", "Contact", or "Projects". Any content without a `date` in its frontmatter is treated as a page.

Edit this file at `content/about/index.md`.
"##;

const INDEX_HTML: &str = r##"{% extends "base.html" %}

{% block content %}
{% if site_description %}
<p class="site-description">{{ site_description }}</p>
{% endif %}
<h2>Posts</h2>
<ul class="post-list">
  {% for post in posts %}
  <li>
    <a href="/posts/{{ post.path }}/">{{ post.title }}</a>
    <time datetime="{{ post.date }}">{{ post.date }}</time>
    {% if post.summary %}<p>{{ post.summary }}</p>{% endif %}
  </li>
  {% endfor %}
</ul>
{% endblock %}
"##;

const GITIGNORE: &str = "dist/\ntarget/\n";

pub fn init(name: Option<&str>) -> anyhow::Result<()> {
    let project_dir = match name {
        Some(n) => {
            let dir = PathBuf::from(n);
            if dir.exists() {
                anyhow::bail!("directory '{}' already exists", dir.display());
            }
            std::fs::create_dir_all(&dir)?;
            dir
        }
        None => std::env::current_dir()?,
    };

    if project_dir.join("kiln.toml").exists() {
        anyhow::bail!("kiln.toml already exists in {}", project_dir.display());
    }

    let title = derive_title(name, &project_dir);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    write_file(
        &project_dir,
        "kiln.toml",
        &KILN_TOML.replace("{{TITLE}}", &title),
    )?;
    write_file(&project_dir, ".gitignore", GITIGNORE)?;
    write_file(&project_dir, "templates/base.html", BASE_HTML)?;
    write_file(&project_dir, "templates/post.html", POST_HTML)?;
    write_file(&project_dir, "templates/page.html", PAGE_HTML)?;
    write_file(&project_dir, "templates/index.html", INDEX_HTML)?;
    write_file(&project_dir, "static/css/main.css", MAIN_CSS)?;
    write_file(
        &project_dir,
        "content/hello/index.md",
        &SAMPLE_POST.replace("{{DATE}}", &today),
    )?;
    write_file(&project_dir, "content/about/index.md", SAMPLE_PAGE)?;

    eprintln!("initialized kiln site in {}", project_dir.display());
    eprintln!("  run `kiln build` to compile, or `kiln dev` to start the dev server");
    Ok(())
}

fn derive_title(name: Option<&str>, project_dir: &Path) -> String {
    let raw = name
        .map(String::from)
        .or_else(|| {
            project_dir
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "My Site".into());

    raw.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_file(base: &Path, relative: &str, content: &str) -> anyhow::Result<()> {
    let path = base.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}
