---
title: "Raw Blocks"
slug: raw-blocks
---

# Raw Blocks

In standard Markdown, fenced code blocks render as `<pre><code>` elements — formatted source code displayed on the page. kiln extends this behavior with **raw blocks**: fenced code blocks marked with the `{=format}` attribute that are passed to a compiler for transformation instead of being rendered as code listings.

## How Raw Blocks Work

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

## Supported Formats

kiln's built-in compiler handles these formats:

| Format    | Attribute    | Purpose                                    |
|-----------|--------------|--------------------------------------------|
| `typst`   | `{=typst}`   | Typst markup, compiled to HTML             |
| `mermaid` | `{=mermaid}` | Mermaid diagram definitions                |
| `html`    | `{=html}`    | Raw HTML, passed through without escaping  |

## Typst Blocks

Typst is a modern typesetting system. kiln ships with built-in Typst compilation, so you can embed Typst content directly in your Markdown posts and pages.

This is useful for mathematical notation, complex tables, styled boxes, and anything that benefits from Typst's powerful layout engine.

### Example

````
```{=typst}
#rect(fill: blue, width: 100%, height: 2em)[Hello from Typst]
```
````

kiln compiles this Typst source into HTML at build time and injects the result into your page. The output replaces the code block entirely — no `<pre>` or `<code>` tags are produced.

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

This produces a rendered HTML table directly in the page, formatted by Typst.

## HTML Blocks

Use `{=html}` to inject raw HTML into your content without Markdown processing or escaping:

````
```{=html}
<div class="custom-widget">
  <iframe src="https://example.com/embed" width="100%" height="400"></iframe>
</div>
```
````

The HTML is inserted into the page output as-is. This is helpful for embedding widgets, iframes, or custom markup that Markdown cannot express.

Note that ` ```html ` (without `{=html}`) renders as a normal code block — useful for showing HTML examples in documentation.

## Mermaid Blocks

Mermaid is a diagramming language for creating flowcharts, sequence diagrams, Gantt charts, and more:

````
```{=mermaid}
graph TD
    A[Write Markdown] --> B[kiln build]
    B --> C[Static Site]
```
````

Mermaid blocks are wrapped in a `<pre class="mermaid">` element for client-side rendering. Include the Mermaid JS library in your `base.html` template:

```html
<script type="module">
  import mermaid from 'https://cdn.jsdelivr.net/npm/mermaid/dist/mermaid.esm.min.mjs';
  mermaid.initialize({ startOnLoad: true });
</script>
```

## Behavior Without a Compiler

When kiln encounters a raw block but no compiler is available for it, the block falls back to a `<pre>` element with a `data-format` attribute:

```html
<pre data-format="typst">
#rect(fill: blue, width: 100%, height: 2em)[Hello from Typst]
</pre>
```

The source content is preserved verbatim. This makes it possible to handle rendering on the client side, or to gracefully degrade when a compiler is not present.

## Attributes on Raw Blocks

Raw blocks support additional brace attributes alongside the `=format` marker. You can attach IDs and classes:

````
```{=typst #fig:diagram .wide}
#rect(fill: green, width: 100%)[A wide diagram]
```
````

These attributes are applied to the output element, allowing you to target specific raw blocks with CSS or reference them with anchor links.

## The `RawBlockCompiler` Trait

For advanced users and tool authors, kiln's document library (`kiln-doc`) defines a `RawBlockCompiler` trait. This is the interface that compilers implement to handle raw blocks.

The kiln binary provides a built-in Typst implementation of this trait. If you are building custom tooling on top of `kiln-doc`, you can implement `RawBlockCompiler` to add support for additional formats — for example, a server-side Mermaid renderer, a LaTeX compiler, or a custom DSL.

```rust
pub trait RawBlockCompiler {
    fn compile(&self, format: &str, source: &str) -> Result<String, String>;
}
```

The `format` parameter receives the format name from the `{=format}` attribute (e.g., `"typst"`), and `source` contains the raw content of the fenced block. The compiler returns an HTML string that replaces the block in the final output.

This design keeps kiln's core Markdown processing decoupled from any specific compilation backend, making it straightforward to extend.
