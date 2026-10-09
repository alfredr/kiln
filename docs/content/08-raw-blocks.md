---
title: "Raw Blocks"
slug: raw-blocks
---

In standard Markdown, fenced code blocks render as `<pre><code>` elements — formatted source code displayed on the page. kiln extends this behavior with **raw blocks**: fenced code blocks marked with the `{=format}` attribute that are passed to a compiler for transformation instead of being rendered as code listings.

## How Raw Blocks Work {#sec:how}

A raw block uses the `{=format}` brace attribute on a fenced code block. The `=` prefix signals that the content should be compiled rather than displayed as code. The format name after `=` tells kiln which compiler to use.

A regular code block:

````
```typst
This renders as a code listing with syntax highlighting.
```
````

A raw block:

````
```{=typst}
This is passed to the Typst compiler and rendered as HTML output.
```
````

The difference is the `{=format}` attribute. Without it, every fenced code block — including ` ```html `, ` ```typst `, and ` ```mermaid ` — renders as a normal code listing.

## Supported Formats {#sec:formats}

kiln's built-in compiler handles these formats:

| Format       | Attribute       | Purpose                                          |
|--------------|-----------------|--------------------------------------------------|
| `typst`      | `{=typst}`      | Typst markup, compiled to inline SVG             |
| `typst-html` | `{=typst-html}` | Typst markup, compiled to HTML (reflowable)      |
| `mermaid`    | `{=mermaid}`    | Mermaid diagrams, compiled to inline SVG         |
| `html`       | `{=html}`       | Raw HTML, passed through without escaping        |

## Typst Blocks {#sec:typst}

Typst is a modern typesetting system. kiln ships with built-in Typst compilation, so you can embed Typst content directly in your Markdown posts and pages.

This is useful for mathematical notation, complex tables, styled boxes, and anything that benefits from Typst's powerful layout engine.

There are two Typst output modes:

- **`{=typst}`** — Compiles to inline SVG. Full visual fidelity: supports `rect`, `circle`, equations, custom layouts, and everything Typst can render. The SVG is embedded directly in the page.
- **`{=typst-html}`** — Compiles to HTML. Produces lighter, reflowable output, but Typst's HTML export is incomplete — visual elements like `rect` and math equations may be dropped.

### Example

````
```{=typst}
#strong[Hello from Typst!] This content is _compiled_ at build time.
```
````

This produces:

```{=typst}
#strong[Hello from Typst!] This content is _compiled_ at build time.
```

kiln compiles this Typst source into an inline SVG at build time and injects the result into your page. The output replaces the code block entirely — no `<pre>` or `<code>` tags are produced.

### A More Involved Example

````
```{=typst}
#let data = (
  ("Category", "Value"),
  ("Alpha", "42"),
  ("Beta", "17"),
  ("Gamma", "89"),
)

#table(
  columns: 2,
  ..data.flatten()
)
```
````

This produces:

```{=typst}
#let data = (
  ("Category", "Value"),
  ("Alpha", "42"),
  ("Beta", "17"),
  ("Gamma", "89"),
)

#table(
  columns: 2,
  ..data.flatten()
)
```

### Typst HTML Mode

Use `{=typst-html}` when you want lighter, reflowable HTML output instead of SVG. This works well for text-heavy content but does not support visual elements like `rect` or equations:

````
```{=typst-html}
#strong[This is bold] and this is _emphasized_ text compiled to HTML.
```
````

This produces:

```{=typst-html}
#strong[This is bold] and this is _emphasized_ text compiled to HTML.
```

Compare this with `{=typst}` (SVG) — the SVG version renders everything Typst supports, while the HTML version is smaller and flows with the page text but may drop visual elements.

## HTML Blocks {#sec:html}

Use `{=html}` to inject raw HTML into your content without Markdown processing or escaping:

````
```{=html}
<div class="custom-widget">
  <iframe src="https://example.com/embed" width="100%" height="400"></iframe>
</div>
```
````

This produces:

```{=html}
<div style="padding: 1em; background: #f0f0f0; border: 1px solid #ccc; border-radius: 4px;">
  <strong>This HTML was injected directly</strong> — no Markdown processing applied.
</div>
```

The HTML is inserted into the page output as-is. This is helpful for embedding widgets, iframes, or custom markup that Markdown cannot express.

Note that ` ```html ` (without `{=html}`) renders as a normal code block — useful for showing HTML examples in documentation.

## Mermaid Blocks {#sec:mermaid}

Mermaid is a diagramming language for creating flowcharts, sequence diagrams, Gantt charts, and more:

````
```{=mermaid}
flowchart LR
    A[Write Markdown] --> B[kiln build]
    B --> C[Static Site]
```
````

This produces:

```{=mermaid}
flowchart LR
    A[Write Markdown] --> B[kiln build]
    B --> C[Static Site]
```

kiln compiles Mermaid diagrams to inline SVG at build time using a native Rust renderer. No client-side JavaScript is required — the diagram is embedded directly in the page, just like Typst blocks.

## Custom Format Aliases {#sec:aliases}

You can define custom format aliases in `kiln.toml` to import Typst packages automatically. For example, a `fletcher` alias for Feynman diagrams:

```toml
[[typst.formats]]
name = "fletcher"
output = "typst"
preamble = '#import "@preview/fletcher:0.5.8" as fletcher: diagram, node, edge'
```

Then use it with `{=fletcher}`:

````
```{=fletcher}
#diagram(
  node-stroke: none,
  edge-stroke: 0.8pt,
  spacing: (3em, 2em),
  node((0, 0), $e^-$),
  node((2, 0), $e^-$),
  node((0, 2), $e^+$),
  node((2, 2), $e^+$),
  edge((0, 0), (1, 1), "-|>-", label: $gamma$),
  edge((1, 1), (2, 0), "-|>-"),
  edge((0, 2), (1, 1), "-|>-"),
  edge((1, 1), (2, 2), "-|>-", label: $gamma$),
)
```
````

This produces:

```{=fletcher}
#diagram(
  node-stroke: none,
  edge-stroke: 0.8pt,
  spacing: (3em, 2em),
  node((0, 0), $e^-$),
  node((2, 0), $e^-$),
  node((0, 2), $e^+$),
  node((2, 2), $e^+$),
  edge((0, 0), (1, 1), "-|>-", label: $gamma$),
  edge((1, 1), (2, 0), "-|>-"),
  edge((0, 2), (1, 1), "-|>-"),
  edge((1, 1), (2, 2), "-|>-", label: $gamma$),
)
```

The alias prepends the `preamble` (the `#import` line) before your content, so you write only the diagram code. kiln downloads the package on first build and caches it locally.

## Behavior Without a Compiler {#sec:no-compiler}

When kiln encounters a raw block but no compiler is available for it, the block falls back to a `<pre>` element with a `data-format` attribute:

```html
<pre data-format="typst">
#strong[Hello from Typst!] This content is _compiled_ at build time.
</pre>
```

The source content is preserved verbatim. This makes it possible to handle rendering on the client side, or to gracefully degrade when a compiler is not present.

## Attributes on Raw Blocks {#sec:attrs}

Raw blocks support additional brace attributes alongside the `=format` marker. You can attach IDs and classes:

````
```{=typst #fig:summary .wide}
#emph[This block] has an ID and a class applied to its output element.
```
````

This produces:

```{=typst #fig:summary .wide}
#emph[This block] has an ID and a class applied to its output element.
```

You can also use the colon shorthand to attach an ID directly to the format marker:

````
```{=typst:#fig:summary .wide}
```
````

This is equivalent to `{=typst #fig:summary .wide}`.

These attributes are applied to the output element, allowing you to target specific raw blocks with CSS or reference them with anchor links.

## The `RawBlockCompiler` Trait {#sec:trait}

For advanced users and tool authors, kiln's document library (`kiln-doc`) defines a `RawBlockCompiler` trait. This is the interface that compilers implement to handle raw blocks.

The kiln binary provides a built-in Typst implementation of this trait. If you are building custom tooling on top of `kiln-doc`, you can implement `RawBlockCompiler` to add support for additional formats — for example, a server-side Mermaid renderer, a LaTeX compiler, or a custom DSL.

```rust
pub trait RawBlockCompiler {
    fn compile(&self, format: &str, source: &str) -> Result<String, String>;
}
```

The `format` parameter receives the format name from the `{=format}` attribute (e.g., `"typst"`), and `source` contains the raw content of the fenced block. The compiler returns an HTML string that replaces the block in the final output.

This design keeps kiln's core Markdown processing decoupled from any specific compilation backend, making it straightforward to extend.
