---
title: "Extensions"
---

Beyond standard markdown, kiln provides several syntax extensions for structured documents: brace attributes for annotating elements, cross-references for linking within a document, citations for academic-style references, and directives for controlling numbering and layout.

## Brace Attributes

Attach metadata to elements using brace-delimited attribute lists: `{#id .class key="value"}`. Attributes can include an ID, one or more classes, and arbitrary key-value pairs.

### Supported Attribute Types

| Syntax | Meaning |
| --- | --- |
| `#my-id` | Sets the element's `id` attribute |
| `.my-class` | Adds a CSS class |
| `key="value"` | Sets an arbitrary attribute (quoted value) |
| `key=value` | Sets an arbitrary attribute (unquoted value, no spaces allowed) |

### On Headings

Place the attribute block at the end of the heading line:

```markdown
## Introduction {#intro .section-heading}
```

This renders with the specified ID and class on the heading element. Use this to create stable anchor targets or to apply specific styles to individual headings.

### On Code Blocks

Place the language and attributes together inside braces on the opening fence:

````markdown
```{rust #lst:fibonacci caption="Fibonacci sequence"}
fn fib(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 1,
        _ => fib(n - 1) + fib(n - 2),
    }
}
```
````

The first bare word inside the braces is the language (for syntax highlighting). The `caption` attribute is particularly useful for code listings that you want to reference elsewhere in the document (see cross-references below).

A bare language without braces (`` ```rust ``) also works and is normalized to `{rust}` internally.

### On Paragraphs

Append the attribute block to the end of a paragraph:

```markdown
This paragraph has special styling. {.callout}
```

The attributes are attached to the `<p>` element in the output.

### Multiple Classes and Mixed Attributes

You can combine multiple IDs, classes, and key-value pairs in a single attribute block:

```markdown
## Results {#results .data .highlight lang="en"}
```

## Cross-References

Reference any element in the document by its ID using the `@kind:label` syntax:

```markdown
As shown in @fig:architecture, the pipeline has three stages.

See @sec:intro for background on the problem.

Applying @eq:energy to this scenario yields the expected result.
```

The reference links to the element whose `id` matches the full `kind:label` string. For example, `@fig:architecture` links to the element with `id="fig:architecture"`.

### Reference Termination

A cross-reference is terminated at the first character that is not alphanumeric, a hyphen, or an underscore. This means you can use references naturally within prose:

```markdown
The value from @eq:quadratic, when substituted back, confirms the root.
```

Here the comma after `@eq:quadratic` is not part of the reference.

### Output

Cross-references currently render as `<a>` elements linking to the target:

```html
<a href="#fig:architecture">fig:architecture</a>
```

## Citations

Cite sources using bracket syntax with `@` keys:

```markdown
Recent work [@smith2024] has demonstrated this approach.
```

For multiple citations in a single bracket, separate them with semicolons:

```markdown
Several studies [@smith2024; @jones2025] support this claim.
```

### Output

Citations render as `<cite>` elements with a `data-key` attribute:

```html
<cite data-key="smith2024">[smith2024]</cite>
```

For multiple citations:

```html
<cite data-key="smith2024">[smith2024]</cite><cite data-key="jones2025">[jones2025]</cite>
```

You can use these `data-key` attributes in your templates or client-side scripts to resolve citations against a bibliography.

## Directives

Directives use the `{@command(args)}` syntax to declare document-level configuration. Place a directive on its own line.

### Series

Declare a numbered series for figures, tables, equations, or any other element kind:

```markdown
{@series(figure, label="Figure", display=arabic)}
```

| Parameter | Description |
| --- | --- |
| First positional | The series ID (e.g. `figure`, `table`, `equation`) |
| `label` | The label prefix shown before the number (e.g. "Figure", "Table") |
| `display` | Numbering style: `arabic`, `hierarchical`, `roman`, or `alpha` |

**Display styles:**

- `arabic` — 1, 2, 3, ...
- `hierarchical` — 1.1, 1.2, 2.1, ... (numbered relative to section depth)
- `roman` — i, ii, iii, ...
- `alpha` — a, b, c, ...

Example declaring multiple series at the top of a document:

```markdown
{@series(figure, label="Fig.", display=arabic)}
{@series(table, label="Table", display=arabic)}
{@series(equation, label="Eq.", display=hierarchical)}
```

### Hints

Set cascading property hints on elements matching a CSS-like selector:

```markdown
{@hint(heading, numbering=true)}
{@hint(.wide, width="100%")}
{@hint(#fig-main, position="top")}
{@hint(code-block[lang="rust"], theme="dark")}
```

Selectors support kind names (bare or alias), classes (`.wide`), IDs (`#intro`), attribute matching (`[lang="rust"]`), combinators (`>`/`~>`), and boolean expressions. See the [Selectors](08-selectors.md) page for full documentation.

#### Using Hints in Brace Attributes

You can place a hint directive inside a brace attribute block to scope it to the current element:

```markdown
## Chapter One {#ch1 {@hint(this, numbering=true)}}
```

## Figure Promotion

When a paragraph contains only an image — no other text, no inline elements — kiln automatically promotes it from a `<p>` containing an `<img>` to a full `<figure>` element:

```markdown
![Performance comparison across implementations](benchmark.png)
```

This produces:

```html
<figure>
  <img src="benchmark.png" alt="Performance comparison across implementations">
  <figcaption>Performance comparison across implementations</figcaption>
</figure>
```

The alt text becomes the `<figcaption>`. If the image appears alongside other content in the paragraph, no promotion occurs and it renders as a standard inline `<img>`.

Combine figure promotion with brace attributes to add an ID for cross-referencing:

```markdown
![Performance comparison](benchmark.png){#fig:benchmark}
```

You can then reference this figure elsewhere with `@fig:benchmark`.

## Escaping \@

Since `@` triggers cross-reference parsing, use a backslash to write a literal `@` character:

```markdown
Contact us at user\@example.com.
```

This renders as plain text without attempting to resolve a reference. The same escaping applies inside any context where `@` would normally be interpreted.
