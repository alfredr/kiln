---
title: "Content"
---

Kiln organizes your writing into two kinds of content: **posts** and **pages**. Both live in a single `content/` directory, which is scanned recursively. The distinction is simple: if a piece of content has a `date` in its frontmatter, it's a post. If it doesn't, it's a page.

## Layouts

Every content entry can use one of two layouts.

### Directory Layout

```
content/my-post/index.md
```

The markdown file must be named `index.md`. Any other files in the same directory (images, data files, PDFs) are **co-located assets** and will be copied alongside the rendered output. This layout is ideal when content includes images or downloadable files.

### Single-File Layout

```
content/my-post.md
```

The filename (without the `.md` extension) becomes the slug. This is the simpler option for content that has no accompanying assets.

## Nested Content

Content directories can be nested to any depth. Subdirectories create a hierarchy — useful for documentation, book-style chapters, or any grouped content.

```
content/
  about.md
  01-guides/
    index.md                  ← section page
    01-getting-started.md     ← child page
    02-deployment.md          ← child page
  02-reference/
    index.md
    01-api/
      index.md                ← nested sub-section
      endpoints.md
    02-cli.md
```

Each directory with an `index.md` is both a page and a section that can contain children. A directory without `index.md` is a **virtual section** — it groups children in the navigation but has no page of its own.

### Path vs. Slug

Every content entry has two identifiers:

- **`slug`** — the local name for this entry (e.g., `"getting-started"`)
- **`path`** — the full URL path from the content root (e.g., `"guides/getting-started"`)

For top-level entries, `path` equals `slug`. For nested entries, `path` includes the parent path segments joined with `/`.

## Frontmatter

Every content file starts with an optional YAML frontmatter block delimited by `---` lines.

```yaml
---
title: "Building a Static Site Generator in Rust"
date: "2026-01-15"
tags:
  - rust
  - web
summary: "A walkthrough of how kiln processes markdown into HTML."
draft: false
slug: "building-ssg"
---
```

All fields are optional:

| Field | Type | Description |
| --- | --- | --- |
| `title` | string | The content title. If omitted, the slug is used as a fallback in templates. |
| `date` | string | Publication date in `YYYY-MM-DD` format. **Entries with a date become posts; entries without become pages.** |
| `tags` | list of strings | Tags for categorization (typically used on posts). |
| `summary` | string | A short description, useful for index pages and feeds. |
| `draft` | boolean | When `true`, the entry is excluded from builds unless `--drafts` is passed. Defaults to `false`. |
| `slug` | string | Override the slug derived from the filename. |

The frontmatter is extensible: any additional YAML keys you include are captured in a map and made available to your templates.

### Slug Customization

By default, the slug is derived from the filename or directory name. You can customize it two ways:

1. **Per-entry**: set `slug` in frontmatter to override the local name.
2. **Site-wide**: set `slug_pattern` in `kiln.toml` with a regex. The first capture group becomes the slug. For example, `slug_pattern = "^\\d+-(.+)$"` strips numeric prefixes like `01-` from filenames.

Frontmatter `slug` takes priority over `slug_pattern`.

### Section Configuration

A section's `index.md` can include special frontmatter keys that configure how its children are handled:

| Key | Type | Description |
| --- | --- | --- |
| `section_slug_pattern` | string | Override `slug_pattern` for this section's children. |

## Posts vs. Pages

The only difference between a post and a page is whether the frontmatter contains a `date` field:

- **Posts** have a `date`. They are rendered with `post.html`, output to `dist/posts/<path>/index.html`, and listed on the index page.
- **Pages** have no `date`. They are rendered with `page.html` and output to `dist/<path>/index.html`.

Both can exist at any level in the content hierarchy.

## Ordering

**Posts** are sorted by date, newest first.

**Pages** are sorted alphabetically by path. If you want to control the order of pages, prefix the directory or filename with a number and use `slug_pattern` to strip the prefix from URLs:

```
content/01-getting-started.md
content/02-content.md
content/03-markdown.md
```

With `slug_pattern = "^\\d+-(.+)$"` in `kiln.toml`, the output paths become `/getting-started/`, `/content/`, `/markdown/` while the filesystem ordering is preserved.

## Slugs and Paths

| Source path | Slug | Path |
| --- | --- | --- |
| `content/my-post/index.md` | `my-post` | `my-post` |
| `content/my-post.md` | `my-post` | `my-post` |
| `content/guides/setup/index.md` | `setup` | `guides/setup` |
| `content/guides/setup.md` | `setup` | `guides/setup` |

## Output Paths

Kiln writes each piece of content to an `index.html` file inside a directory named after the path:

| Content type | Output path |
| --- | --- |
| Post with path `my-post` | `dist/posts/my-post/index.html` |
| Page with path `about` | `dist/about/index.html` |
| Nested page with path `guides/setup` | `dist/guides/setup/index.html` |

This produces clean URLs: `/guides/setup/` rather than `/guides/setup.html`.

## Co-located Assets

When you use the directory layout, every file in the directory besides `index.md` is treated as a co-located asset. These files are copied into the output directory next to the rendered HTML. Child content directories are **not** copied as assets — only non-content files are.

For example, given:

```
content/my-post/
  index.md
  diagram.png
  data.csv
```

The build produces:

```
dist/posts/my-post/
  index.html
  diagram.png
  data.csv
```

You can reference co-located assets with relative paths in your markdown:

```markdown
![Architecture diagram](diagram.png)
```

## Drafts

Mark content as a draft by setting `draft: true` in its frontmatter:

```yaml
---
title: "Work in Progress"
date: "2026-03-01"
draft: true
---
```

Drafts are **excluded** from the build output by default. To include them, pass the `--drafts` flag:

```sh
kiln build --drafts
kiln dev --drafts
```

If a section's `index.md` is marked as a draft, the entire section and all its children are excluded.
