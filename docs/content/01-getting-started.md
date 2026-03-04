---
title: "Getting Started"
---

# Getting Started

This guide walks you through installing kiln, creating your first site, building it, and previewing it locally.

## Installation

Kiln is not yet published to crates.io, so you need to build it from source. Make sure you have a working [Rust toolchain](https://rustup.rs/) installed, then run:

```sh
cargo install --path crates/kiln
```

This compiles kiln and places the binary in your Cargo bin directory (typically `~/.cargo/bin/`). Verify the installation:

```sh
kiln --help
```

## Create a Site

Scaffold a new site with `kiln init`:

```sh
kiln init my-site
```

This creates the following directory structure:

```
my-site/
  kiln.toml
  .gitignore
  templates/
    base.html
    post.html
    page.html
    index.html
  static/
    css/
      main.css
  content/
    hello/
      index.md       # A sample post (has a date)
    about/
      index.md       # A sample page (no date)
```

**`kiln.toml`** is the site configuration file. It lives at the root of your project and defines metadata such as the site title and base URL.

**`templates/`** contains the Tera templates that control how your content is rendered into HTML. The scaffolded templates give you a working starting point that you can customize freely.

**`static/`** holds files that are copied verbatim into the output directory. Put your CSS, JavaScript, fonts, and other assets here.

**`content/`** is where all your content lives — both posts and pages. Entries with a `date` in their frontmatter are posts; entries without are pages.

## Build

To build your site, run:

```sh
kiln build
```

Kiln reads `kiln.toml`, processes every entry in `content/`, renders them through the corresponding templates (posts through `post.html`, pages through `page.html`), and writes the final HTML to the `dist/` directory.

The output structure mirrors your content:

- Posts are written to `dist/posts/<slug>/index.html`
- Pages are written to `dist/<slug>/index.html`
- Static files are copied to `dist/` preserving their directory structure

## Dev Server

During development, use the built-in dev server instead of rebuilding manually:

```sh
kiln dev
```

This starts a local HTTP server with **live reload** — when you edit a content file, page, template, or static asset, kiln automatically rebuilds and refreshes your browser.

By default the server listens on port 3000. Change it with the `--port` flag:

```sh
kiln dev --port 8080
```

To include draft posts in the preview, pass the `--drafts` flag:

```sh
kiln dev --drafts
```

## CLI Reference

Below is a summary of kiln's commands and flags.

### Global Flags

| Flag | Description |
| --- | --- |
| `--root <PATH>` | Set the project root directory. Defaults to the current working directory. |

### Commands

| Command | Description |
| --- | --- |
| `kiln init <NAME>` | Scaffold a new site in a directory called `<NAME>`. |
| `kiln build` | Build the site and write output to `dist/`. |
| `kiln dev` | Start the development server with live reload. |

### `kiln build` Flags

| Flag | Description |
| --- | --- |
| `--drafts` | Include posts marked as drafts in the output. |

### `kiln dev` Flags

| Flag | Description |
| --- | --- |
| `--port <N>` | Port for the dev server (default: 3000). |
| `--drafts` | Include posts marked as drafts in the preview. |
