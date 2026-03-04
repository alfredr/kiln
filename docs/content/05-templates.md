---
title: "Templates"
---

kiln uses [Tera](https://keats.github.io/tera/), a Jinja2-like template engine for Rust. Templates control the HTML structure of every page on your site. They live in the `templates/` directory (configurable via `kiln.toml`) and use standard Tera syntax: `{{ variables }}`, `{% blocks %}`, `{# comments #}`, filters, and template inheritance.

## Required Template Files

kiln expects four template files in your `templates/` directory:

| File         | Purpose                              |
|--------------|--------------------------------------|
| `base.html`  | Master layout shared by all pages    |
| `post.html`  | Renders individual blog posts        |
| `page.html`  | Renders standalone pages             |
| `index.html` | The homepage and post listing        |

All four are required. `post.html`, `page.html`, and `index.html` typically extend `base.html` using Tera's `{% extends %}` mechanism.

## Template Variables

Every template receives a set of site-wide variables. Depending on the template, additional context-specific variables are also available.

### Site Variables (All Templates)

These are available in every template:

- `site_title` — the site title from `kiln.toml`
- `site_url` — the site base URL
- `site_description` — the site description
- `site_author` — the site author

### Navigation Variables (All Templates)

Every template also receives navigation data:

- `posts` — a flat array of all posts, each with: `path`, `slug`, `title`, `date`, `summary`, `tags`
- `pages` — a flat array of all pages, each with: `path`, `slug`, `title`
- `nav_tree` — a recursive tree of all content (see [Tree Navigation](#tree-navigation) below)

### `post.html` Variables

In addition to site and navigation variables, `post.html` receives:

- `title` — the post title from frontmatter
- `date` — the post date as a string
- `tags` — an array of tag strings (e.g., `["rust", "web"]`)
- `summary` — the post summary text
- `path` — the full URL path for the post (e.g., `"guides/my-post"`)
- `slug` — the local slug for the post (e.g., `"my-post"`)
- `body` — the rendered HTML content of the post

### `page.html` Variables

In addition to site and navigation variables, `page.html` receives:

- `title` — the page title from frontmatter
- `path` — the full URL path for the page
- `slug` — the local slug for the page
- `body` — the rendered HTML content of the page

### `index.html` Variables

The index template receives site and navigation variables only. Use `posts` and `pages` to build your homepage.

## Tree Navigation

The `nav_tree` variable provides a recursive tree structure for building hierarchical navigation (sidebars, breadcrumbs, table of contents). Each node has:

- `path` — full URL path for links
- `slug` — local name
- `title` — display title
- `has_page` — `true` if this node has a rendered page, `false` for virtual sections
- `is_post` — `true` for posts, `false` for pages and sections
- `children` — array of child nodes (same structure, recursive)
- `number` — hierarchical chapter number (e.g., `"2.3"`)

Use a Tera macro for recursive rendering:

```html
{% macro render_nav(nodes) %}
<ul>
  {% for node in nodes %}
  <li>
    {% if node.has_page %}
      <a href="/{{ node.path }}/">{{ node.title }}</a>
    {% else %}
      <span>{{ node.title }}</span>
    {% endif %}
    {% if node.children %}
      {{ self::render_nav(children=node.children) }}
    {% endif %}
  </li>
  {% endfor %}
</ul>
{% endmacro render_nav %}
```

For flat sites, `nav_tree` contains the same entries as `pages` and `posts` but in a tree structure. For nested content, it reflects the directory hierarchy.

## The `| safe` Filter

The `body` variable in `post.html` and `page.html` contains pre-rendered HTML. By default, Tera escapes HTML entities to prevent injection. You **must** use the `| safe` filter when outputting body content, otherwise your rendered markup will display as escaped text:

```html
<!-- Correct: HTML renders properly -->
{{ body | safe }}

<!-- Wrong: you'll see raw <p>, <h1>, etc. on the page -->
{{ body }}
```

## Example Templates

### `base.html`

A minimal base layout that defines the outer HTML structure and two blocks for child templates to override:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{% block title %}{{ site_title }}{% endblock %}</title>
  <meta name="description" content="{{ site_description }}">
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <header>
    <nav>
      <a href="/">{{ site_title }}</a>
    </nav>
  </header>

  <main>
    {% block content %}{% endblock %}
  </main>

  <footer>
    <p>&copy; {{ site_author }}</p>
  </footer>
</body>
</html>
```

### `post.html`

Extends the base layout and fills in both blocks with post-specific content:

```html
{% extends "base.html" %}

{% block title %}{{ title }} | {{ site_title }}{% endblock %}

{% block content %}
<article>
  <h1>{{ title }}</h1>
  <time>{{ date }}</time>

  {% if tags %}
  <ul class="tags">
    {% for tag in tags %}
    <li>{{ tag }}</li>
    {% endfor %}
  </ul>
  {% endif %}

  <div class="post-body">
    {{ body | safe }}
  </div>
</article>
{% endblock %}
```

### `page.html`

Similar to post.html but simpler, since pages don't have dates or tags:

```html
{% extends "base.html" %}

{% block title %}{{ title }} | {{ site_title }}{% endblock %}

{% block content %}
<article>
  <h1>{{ title }}</h1>
  <div class="page-body">
    {{ body | safe }}
  </div>
</article>
{% endblock %}
```

### `index.html`

The homepage template loops over all published posts:

```html
{% extends "base.html" %}

{% block content %}
<h1>{{ site_title }}</h1>

{% if site_description %}
<p class="site-description">{{ site_description }}</p>
{% endif %}

<ul class="post-list">
  {% for post in posts %}
  <li>
    <a href="/posts/{{ post.path }}/">
      <h2>{{ post.title }}</h2>
    </a>
    <time>{{ post.date }}</time>
    {% if post.summary %}
    <p>{{ post.summary }}</p>
    {% endif %}
  </li>
  {% endfor %}
</ul>
{% endblock %}
```

## Customization

Templates are fully yours to modify. kiln imposes no constraints beyond the four required files and the available variables. Here are some ideas:

- **Add CSS frameworks** — link to a stylesheet in `base.html`, place the CSS file in `static/`
- **Dynamic navigation** — use the `pages` or `nav_tree` variables to generate menus automatically
- **Active page highlighting** — compare `page.path == path` to highlight the current page in nav
- **Conditional blocks** — use `{% if %}` to show elements only when data exists (e.g., show tags only when a post has them)
- **Partials** — use `{% include "header.html" %}` to break large templates into reusable fragments
- **Custom filters** — Tera supports built-in filters like `| upper`, `| truncate`, `| date` and more
- **Static assets** — anything in `static/` is copied to the output root, so reference images, fonts, and scripts with absolute paths like `/logo.png`

The only rule: use `{{ body | safe }}` when rendering content bodies, and make sure your four template files are present.
