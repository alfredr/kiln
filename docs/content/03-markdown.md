---
title: "Markdown Syntax"
---

Kiln uses its own markdown parser built from the CommonMark spec with several extensions. This page is a reference for every supported syntax element.

## Headings {#sec:headings}

Use `#` characters at the start of a line to create headings, from level 1 through level 6.

```markdown
# Heading 1
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6
```

Headings create nested `<section>` wrappers in the output. Each heading and the content that follows it (up to the next heading of equal or higher level) are wrapped in a `<section>` element. This makes it straightforward to style or target specific sections of a document with CSS.

## Paragraphs

Paragraphs are separated by one or more blank lines. Consecutive lines of text without a blank line between them form a single paragraph.

```markdown
This is the first paragraph.

This is the second paragraph.
```

## Line Breaks

There are three ways to handle line breaks within a paragraph:

**Hard break with trailing spaces.** End a line with two or more spaces to insert a `<br>`:

```markdown
This line has a hard break
and continues here.
```

**Hard break with backslash.** Place a `\` immediately before the newline:

```markdown
This line has a hard break\
and continues here.
```

**Soft break.** A single newline without trailing spaces or a backslash is treated as a soft break — the browser renders it as a space:

```markdown
These two lines
become one paragraph.
```

## Emphasis

Wrap text in single asterisks or underscores for *italic* emphasis:

```markdown
*italic text*
_also italic text_
```

Both produce `<em>` in the output.

## Strong Emphasis

Wrap text in double asterisks or underscores for **bold** emphasis:

```markdown
**bold text**
__also bold text__
```

Both produce `<strong>` in the output.

## Strong and Emphasis Combined

Use triple asterisks for ***bold and italic*** simultaneously:

```markdown
***bold and italic***
```

This nests `<strong>` inside `<em>` (or vice versa).

## Strikethrough

Wrap text in double tildes to strike it through:

```markdown
~~deleted text~~
```

This renders as `<del>deleted text</del>`.

## Inline Code

Wrap text in backticks for inline code:

```markdown
Use the `kiln build` command.
```

This renders as `<code>kiln build</code>`. If the code itself contains backticks, use double backticks as the delimiter:

```markdown
`` code with a ` backtick ``
```

## Links

Create links with the standard `[text](url)` syntax:

```markdown
[Kiln on GitHub](https://github.com/example/kiln)
```

This renders as an `<a>` element. You can also add a title:

```markdown
[Kiln on GitHub](https://github.com/example/kiln "Visit the repo")
```

## Images {#sec:images}

Insert images with the `![alt](src)` syntax:

```markdown
![A diagram of the build pipeline](pipeline.png)
```

**Figure promotion.** When a paragraph contains *only* an image and nothing else, kiln automatically promotes it to a `<figure>` element with a `<figcaption>` derived from the alt text. This means:

```markdown
![Architecture overview](architecture.png)
```

produces:

```html
<figure>
  <img src="architecture.png" alt="Architecture overview">
  <figcaption>Architecture overview</figcaption>
</figure>
```

If the image appears inline alongside other text, it stays as a plain `<img>` within the paragraph.

## Unordered Lists

Start lines with `-`, `*`, or `+` followed by a space:

```markdown
- First item
- Second item
- Third item
```

All three markers are interchangeable. Nested lists are created by indenting:

```markdown
- Outer item
  - Inner item
  - Another inner item
- Back to outer
```

## Ordered Lists

Start lines with a number, a period, and a space:

```markdown
1. First item
2. Second item
3. Third item
```

The start number of the first item is honored. If you begin with `5.`, the rendered `<ol>` will have `start="5"`. Subsequent numbers do not need to be sequential — the browser increments automatically — but using correct numbers improves readability of the source.

## Block Quotes

Prefix lines with `>` to create a block quote:

```markdown
> This is a block quote.
> It can span multiple lines.
```

Block quotes support **lazy continuation**: once a `>` line starts a quote, subsequent lines without the `>` prefix are still included until a blank line ends the quote.

```markdown
> This is the start of a quote.
The parser continues including this line.

This line is outside the quote.
```

Block quotes nest by stacking `>` characters:

```markdown
> Outer quote
>
> > Nested quote
```

## Code Blocks {#sec:code-blocks}

Fence a block of code with triple backticks or triple tildes. Add a language identifier after the opening fence for syntax highlighting:

````markdown
```rust
fn main() {
    println!("Hello, kiln!");
}
```
````

You can also use tildes:

```markdown
~~~python
print("Hello, kiln!")
~~~
```

kiln automatically applies syntax highlighting at build time — no client-side JavaScript is needed. The highlighting uses the base16-ocean.dark color scheme with inline styles, so code blocks are colorized in the static HTML output. If a language identifier is not recognized, the code is rendered as plain text.

For example, this Rust code block is highlighted automatically:

```rust
fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}
```

## Thematic Breaks

Create a horizontal rule with three or more hyphens, asterisks, or underscores on a line by themselves:

```markdown
---
***
___
```

Each produces a `<hr>` element. You can use spaces between the characters if you prefer: `- - -` also works.

## Math {#sec:math}

Kiln recognizes math between dollar signs. It does not interpret what is inside them: how math renders, and so which syntax you write it in, is configuration (see [Rendering Math](#sec:math-render) below).

**Inline math.** Wrap an expression in single dollar signs:

```markdown
Einstein's famous equation $E=mc^2$ relates energy and mass.
```

The content between the dollar signs is placed inside an inline math element.

**Display math.** Use double dollar signs on their own lines for a block-level equation:

```markdown
$$
\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}
$$
```

Display math renders as a block element, centered on its own line.

### Rendering Math {#sec:math-render}

By default kiln writes math out as its source, inside `<span class="math">` or `<div class="math display">`. That suits a client-side renderer such as KaTeX or MathJax, loaded from your templates.

To typeset math at build time instead, give it a `format` with a hint in `kiln.toml`. Kiln then compiles it the way it compiles a [raw block](/raw-blocks/), using a format alias whose `template` turns the source into Typst:

```toml
[[hints]]
selector = 'math[display="false"]'
format = "math"

[[hints]]
selector = 'math[display="true"]'
format = "math-display"

[[typst.formats]]
name = "math"
template = "${source}$"

[[typst.formats]]
name = "math-display"
template = "$ {source} $"
```

With this configuration, math is written in Typst's syntax: `$sqrt(x^2 + 1)$` rather than `$\sqrt{x^2 + 1}$`. Inline math becomes an SVG sized in em that sits on the baseline of the surrounding text and takes its color. The source stays in the page as visually hidden text, so it can still be copied, found and read by screen readers.

Math is not special in this: any inline code (`code-inline`) given a `format` compiles the same way.

## Escaping Special Characters {#sec:escaping}

Prefix any special character with a backslash to render it literally:

```markdown
\* This is not italic \*
\@ This is not a cross-reference
\$ This is not math
\# This is not a heading
```

This works for all ASCII punctuation characters. It is particularly useful for characters that kiln extends with special meaning, such as `@` (cross-references) and `$` (math).
