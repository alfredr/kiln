---
title: "Extensions"
---

{@content(heading, "{number}. ")}

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

Cross-references render as `<a>` elements linking to the target. Without a series label, the raw target string is used:

```html
<a href="#fig:architecture">fig:architecture</a>
```

When the target element belongs to a series with a `label` init, the label template is used instead, with `{n}` replaced by the computed number:

```html
<a href="#fig:architecture">Figure 1</a>
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
{@series(fig, counts=figure, formats=["1"])}
{@init(figure, label="Figure {n}")}
```

| Parameter | Description | Default |
| --- | --- | --- |
| First positional | The series ID (e.g. `fig`, `sec`, `equation`) | required |
| `counts` | Selector matching elements to number — shorthand for `@init(SELECTOR, series="ID")` | — |
| `formats` | Format token array, one per depth level | `["1"]` |
| `parent` | Parent series ID — resets this counter when the parent bumps | — |
| `cycle` | Cycle format list (`true`) or repeat last element (`false`) | `false` |

The `counts` parameter is syntactic sugar. Writing `{@series(fig, counts=figure)}` is equivalent to writing `{@series(fig)}` plus `{@init(figure, series="fig")}`. The label shown in cross-references is set separately via an init with the `label` property and a `{n}` placeholder for the computed number.

**Format tokens:**

| Token | Style | Example output |
| --- | --- | --- |
| `"1"` | Arabic | 1, 2, 3, ... |
| `"I"` | Roman upper | I, II, III, ... |
| `"i"` | Roman lower | i, ii, iii, ... |
| `"A"` | Alpha upper | A, B, C, ... |
| `"a"` | Alpha lower | a, b, c, ... |
| `"(a)"` | Wrapped | (a), (b), (c), ... |

**Numbering modes:**

- **Flat** — A single format token produces a flat counter: 1, 2, 3, ...
- **Hierarchical** — Multiple format tokens on a heading series map depth levels to tokens, joined with dots. For example, `formats=["I","1"]` numbers H1 as I, II and H2 as I.1, I.2, II.1.
- **Parent-child** — When a series has a `parent`, its flat counter resets to zero each time the parent series increments. This lets figures restart numbering per chapter, for instance.

When a series has more depth levels than format tokens, the last token repeats (or cycles if `cycle=true`).

**Examples:**

Flat figure numbering with a label for cross-references:

```markdown
{@series(fig, counts=figure)}
{@init(figure, label="Figure {n}")}

![Architecture](arch.png){#fig:arch}

See @fig:arch for the overview.
```

Hierarchical section numbering with Roman numerals for chapters and Arabic for subsections:

```markdown
{@series(sec, counts=heading, formats=["I","1"])}
```

Figure numbering that resets per chapter:

```markdown
{@series(sec, counts=heading, formats=["I","1"])}
{@series(fig, counts=figure, parent=sec)}
{@init(figure, label="Figure {n}")}
```

### Init

Set cascading property defaults on elements matching a CSS-like selector. Inits use "set if not exists" — authored values on individual nodes are never overwritten.

```markdown
{@init(heading, numbering=true)}
{@init(.wide, width="100%")}
{@init(#fig-main, position="top")}
{@init(code-block[lang="rust"], theme="dark")}
{@init(figure, label="Figure {n}")}
```

The `label` property sets the display text for cross-references. The `{n}` placeholder is replaced with the element's computed series number.

Selectors support kind names (bare or alias), classes (`.wide`), IDs (`#intro`), attribute matching (`[lang="rust"]`), combinators (`>`/`~>`), and boolean expressions. See the [Selectors](/selectors/) page for full documentation.

#### Using Init in Brace Attributes

You can place an init directive inside a brace attribute block to scope it to the current element:

```markdown
## Chapter One {#ch1 {@init(this, numbering=true)}}
```

### Var

Declare a running-state variable for use in `@on` triggers:

```markdown
{@var(fig, 0, format="1")}
{@var(sec, [], format=["I","1"])}
{@var(flag, false)}
```

| Parameter | Description | Default |
| --- | --- | --- |
| First positional | Variable name | required |
| Second positional | Initial value: `0` (flat counter), `[]` (hierarchical counter), `true`/`false` (boolean) | `0` |
| `format` / `formats` | Format token(s) for rendering via `@store` | `["1"]` |
| `cycle` | Cycle format list or repeat last element | `false` |

The initial value determines the variable's type. A `Number` starts at zero and increments by 1 with `@inc`. A `List` starts empty and extends/truncates by heading depth with `@inc`.

### On

Bind actions to nodes matching a selector. Actions fire in declaration order during a document-order tree walk:

```markdown
{@on(figure, @inc(fig), @store(number, fig))}
{@on(heading, @inc(sec), @store(number, sec), @reset(fig))}
{@on(heading, @prepend(@text("{number}. ")))}
```

The first argument is a CSS-like selector (kind names, classes, IDs, attribute matchers, combinators). All subsequent `@`-prefixed arguments are actions.

**Available actions:**

| Action | Effect |
| --- | --- |
| `@inc(var)` | Number: add 1. List: bump at heading depth, truncate deeper levels. |
| `@reset(var)` | Reset variable to its initial value. |
| `@store(prop, var)` | Format the variable using its declared format tokens and store the result as a node property. |
| `@set(prop, value)` | Set a node property to a literal value. |
| `@prepend(@text("tpl"))` | Insert a text node before existing children. |
| `@append(@text("tpl"))` | Insert a text node after existing children. |

Text templates support `{key}` interpolation from node properties. For example, `{number}` reads the node's `number` property. Missing keys interpolate as empty strings.

### Content

Shorthand for `@on(SELECTOR, @prepend(@text(TEMPLATE)))` — prepend formatted text to matching nodes:

```markdown
{@content(heading, "{number}. ")}
```

This is equivalent to:

```markdown
{@on(heading, @prepend(@text("{number}. ")))}
```

### Composing a complete numbering scheme

The `@series` directive is sugar that expands to `@var` + `@init` + `@on`. The primitives can also be used directly for full control. Here are several complete examples showing both forms.

**Flat figure numbering with cross-reference labels:**

Using `@series` sugar:

```markdown
{@series(fig, counts=figure)}
{@init(figure, label="Figure {n}")}

![Architecture](arch.png){#fig:arch}

![Pipeline](pipeline.png){#fig:pipeline}

See @fig:arch and @fig:pipeline.
```

Equivalent using primitives:

```markdown
{@var(fig, 0, format="1")}
{@init(figure, series="fig")}
{@init(figure, label="Figure {n}")}
{@on([series="fig"], @inc(fig), @store(number, fig))}

![Architecture](arch.png){#fig:arch}

![Pipeline](pipeline.png){#fig:pipeline}

See @fig:arch and @fig:pipeline.
```

Result: figures are numbered 1, 2. Cross-references render as "Figure 1" and "Figure 2".

**Hierarchical section numbering with content transformation:**

```markdown
{@series(sec, counts=heading, formats=["I","1"])}
{@init(heading, label="Section {n}")}
{@content(heading, "{number}. ")}

# Introduction {#sec:intro}

## Background {#sec:bg}

## Methods {#sec:methods}

# Results {#sec:results}

See @sec:intro and @sec:methods.
```

Result:

- `# Introduction` renders as `<h1>I. Introduction</h1>`, cross-ref "Section I"
- `## Background` renders as `<h2>I.1. Background</h2>`, cross-ref "Section I.1"
- `## Methods` renders as `<h2>I.2. Methods</h2>`, cross-ref "Section I.2"
- `# Results` renders as `<h1>II. Results</h1>`, cross-ref "Section II"

**Figure numbering that resets per chapter:**

```markdown
{@series(sec, counts=heading, formats=["I","1"])}
{@series(fig, counts=figure, parent=sec)}
{@init(figure, label="Figure {n}")}
{@init(heading, label="Section {n}")}
{@content(heading, "{number}. ")}

# Introduction {#sec:intro}

![Arch](arch.png){#fig:arch}

![Flow](flow.png){#fig:flow}

# Results {#sec:results}

![Data](data.png){#fig:data}

See @fig:arch (Figure 1 in Chapter I) and @fig:data (Figure 1 in Chapter II).
```

The `parent=sec` parameter causes the figure counter to reset to zero each time the section counter increments. Figures in Chapter I are numbered 1, 2. After "# Results" bumps the section counter, figures restart at 1.

**Three-level legal document numbering:**

```markdown
{@series(sec, counts=heading, formats=["I","1","a"])}
{@content(heading, "{number}. ")}

# Part One

## Section One

### Subsection

## Section Two

# Part Two
```

Result: I, I.1, I.1.a, I.2, II. When a heading at level N appears, all deeper levels are truncated — so `## Section Two` after `### Subsection` resets the third level.

**Individual node overrides with `@init`:**

`@init` uses "set if not exists", so authored values on individual nodes always win:

```markdown
{@init(figure, label="Figure {n}")}

![Normal](a.png){#fig:normal}

![Special](b.png){#fig:special label="Diagram {n}"}
```

`@fig:normal` gets "Figure 1", but `@fig:special` gets "Diagram 2" because the authored `label` takes precedence over the init default.

**Custom actions with `@set`:**

Use `@set` to attach arbitrary properties to matching nodes:

```markdown
{@on(figure, @set(loading, lazy))}
{@on(code-block[lang="rust"], @set(theme, monokai))}
```

This sets `loading="lazy"` on every figure and `theme="monokai"` on Rust code blocks, which your templates or CSS can consume.

## Prelude

kiln ships a built-in prelude that defines standard series for sections and figures with cross-reference labels. The prelude is automatically loaded for every document and provides these definitions:

```
@series(sec, counts=heading, formats=["I","1"])
@series(fig, counts=figure, parent=sec)
@init(figure, label="Figure {n}")
@init(heading, label="Section {n}")
```

This means you can use `@fig:name` and `@sec:name` cross-references out of the box — the prelude sets up the counters and label templates. To display visible section numbers in headings, add a `@content` directive to your document:

```markdown
{@content(heading, "{number}. ")}

# Introduction {#sec:intro}

## Background {#sec:bg}
```

### `.kiln` file format

The prelude is a `.kiln` file — plain text with one directive per line and `#` for comments:

```
# My custom prelude
@series(sec, counts=heading, formats=["1","a"])
@series(fig, counts=figure)
@init(figure, label="Fig. {n}")
@init(heading, label="§{n}")
```

### Overriding the prelude

To use a custom prelude instead of the built-in one, set the `prelude` field in `kiln.toml`:

```toml
[build]
prelude = "my-prelude.kiln"
```

The path is relative to the project root. The custom file completely replaces the built-in prelude.

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
