---
title: "Markdown Syntax"
---

# Markdown Syntax

Kiln uses its own markdown parser built from the CommonMark spec with several extensions. This page is a reference for every supported syntax element.

## Headings

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

## Images

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

## Code Blocks

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

The language identifier is added as a class on the rendered `<code>` element (e.g. `class="language-rust"`), which client-side syntax highlighters can use.

## Thematic Breaks

Create a horizontal rule with three or more hyphens, asterisks, or underscores on a line by themselves:

```markdown
---
***
___
```

Each produces a `<hr>` element. You can use spaces between the characters if you prefer: `- - -` also works.

## Math

Kiln supports LaTeX-style math syntax.

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

Display math renders as a block element, centered on its own line. You can pair this with a math rendering library like KaTeX or MathJax in your templates.

## Escaping Special Characters

Prefix any special character with a backslash to render it literally:

```markdown
\* This is not italic \*
\@ This is not a cross-reference
\$ This is not math
\# This is not a heading
```

This works for all ASCII punctuation characters. It is particularly useful for characters that kiln extends with special meaning, such as `@` (cross-references) and `$` (math).
