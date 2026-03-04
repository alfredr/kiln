---
title: "Configuration"
---

kiln is configured through a single file, `kiln.toml`, placed at the root of your project. This file uses [TOML](https://toml.io/) syntax and has two sections: `[site]` for metadata and `[build]` for directory layout. Command-line flags provide additional runtime options.

## The `[site]` Section

The `[site]` section defines your site's metadata. These values are passed to every template as variables.

| Field         | Required | Default                    | Template Variable    |
|---------------|----------|----------------------------|----------------------|
| `title`       | Yes      | —                          | `site_title`         |
| `url`         | No       | `"http://localhost:3000"`  | `site_url`           |
| `description` | No       | `""`                       | `site_description`   |
| `author`      | No       | `""`                       | `site_author`        |

Only `title` is required. The rest have sensible defaults for local development. When deploying, you should set `url` to your production domain.

## The `[build]` Section

The `[build]` section controls where kiln looks for source files and where it writes output. This entire section is optional — if omitted, all fields use their defaults.

| Field          | Default       | Purpose                                      |
|----------------|---------------|----------------------------------------------|
| `content_dir`  | `"content"`   | Directory containing all content (posts and pages) |
| `output_dir`   | `"dist"`      | Directory where the built site is written     |
| `template_dir` | `"templates"` | Directory containing Tera template files      |
| `static_dir`   | `"static"`    | Static assets copied verbatim to output root  |
| `slug_pattern` | —             | Regex to transform filenames into slugs       |

All paths are relative to the project root. The content directory is scanned recursively — subdirectories create hierarchical content sections.

### `slug_pattern`

A regex applied to each filename/dirname to derive the slug. The first capture group `(...)` is used as the slug. If the regex doesn't match a name, the original name is kept.

```toml
[build]
# Strip numeric prefixes: "01-getting-started" → "getting-started"
slug_pattern = "^\\d+-(.+)$"
```

This is useful for controlling filesystem sort order (via numeric prefixes) while keeping clean URLs. Individual entries can override the slug via the `slug` frontmatter field, which takes priority over `slug_pattern`.

Sections can override the pattern for their children by setting `section_slug_pattern` in the section's `index.md` frontmatter.

## CLI Flags

kiln's command-line interface provides flags that override or supplement the configuration file.

### Global Flag

- `--root PATH` — set the project root directory. Defaults to the current working directory. kiln will look for `kiln.toml` and all configured directories relative to this path.

```sh
# Build a site located in another directory
kiln build --root /path/to/my-site
```

### Build and Dev Flags

- `--drafts` — include content marked as drafts in the output. Available for both `build` and `dev` commands.

```sh
# Build the site including drafts
kiln build --drafts

# Run the dev server with drafts visible
kiln dev --drafts
```

### Dev-Only Flag

- `--port N` — set the port for the development server. Defaults to `3000`.

```sh
# Run the dev server on port 8080
kiln dev --port 8080
```

## Complete Annotated Example

Here is a `kiln.toml` with every available option and comments explaining each one:

```toml
[site]
# Required: your site's title, available as {{ site_title }} in templates
title = "My Tech Blog"

# The base URL of your site. Used for generating absolute links.
# Set this to your production domain when deploying.
url = "https://example.com"

# A short description of the site, available as {{ site_description }}
description = "Thoughts on Rust, systems programming, and open source"

# The site author's name, available as {{ site_author }}
author = "Jane Developer"

[build]
# Where your content lives (both posts and pages, relative to project root)
# Scanned recursively — subdirectories create content hierarchy
content_dir = "content"

# Where the built HTML site is written
output_dir = "dist"

# Where your Tera templates live
template_dir = "templates"

# Static files (CSS, images, fonts) copied to the output root
static_dir = "static"

# Regex to transform filenames into slugs (first capture group is used)
# This strips numeric prefixes used for ordering
slug_pattern = "^\\d+-(.+)$"
```

## Minimal Example

If you just want to get started, the only thing you need is a title:

```toml
[site]
title = "My Blog"
```

Everything else falls back to defaults. This is perfectly valid for local development — kiln will look for content in `content/`, templates in `templates/`, and static files in `static/`, writing output to `dist/`. The dev server will run at `http://localhost:3000`.

## Tips

- **Keep `url` as localhost during development.** Only set it to your production URL when you are ready to deploy, or use environment-specific config files.
- **The `output_dir` is fully replaced on each build.** Do not store anything important in it. kiln will clean and regenerate it every time you run `kiln build`.
- **Use `--root` for scripting.** If you have a CI/CD pipeline or a Makefile, `--root` lets you invoke kiln from any working directory.
- **The `static/` directory is copied as-is.** A file at `static/css/style.css` will appear at `/css/style.css` in the built site. No processing is applied to static files.
- **Use `slug_pattern` for clean URLs.** Prefix content files with numbers for ordering (e.g., `01-intro.md`, `02-setup.md`), then set `slug_pattern = "^\\d+-(.+)$"` to strip the numbers from URLs.
