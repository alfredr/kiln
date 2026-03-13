---
title: "Brace Attributes"
---

Attach metadata to elements using brace-delimited attribute lists: `{#id .class key="value"}`. Attributes can include an ID, one or more classes, and arbitrary key-value pairs.

## Supported Attribute Types {#sec:types}

| Syntax | Meaning |
| --- | --- |
| `#my-id` | Sets the element's `id` attribute |
| `.my-class` | Adds a CSS class |
| `key="value"` | Sets an arbitrary attribute (quoted value) |
| `key=value` | Sets an arbitrary attribute (unquoted value, no spaces allowed) |

## On Headings {#sec:on-headings}

Place the attribute block at the end of the heading line:

```markdown
## Introduction {#intro .section-heading}
```

This renders with the specified ID and class on the heading element. Use this to create stable anchor targets or to apply specific styles to individual headings.

## On Code Blocks {#sec:on-code-blocks}

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

The first bare word inside the braces is the language (for syntax highlighting). The `caption` attribute is particularly useful for code listings that you want to reference elsewhere in the document (see [Cross-References](/cross-references/)).

A bare language without braces (`` ```rust ``) also works and is normalized to `{rust}` internally.

## On Paragraphs {#sec:on-paragraphs}

Append the attribute block to the end of a paragraph:

```markdown
This paragraph has special styling. {.callout}
```

The attributes are attached to the `<p>` element in the output.

## Multiple Classes and Mixed Attributes {#sec:multiple}

You can combine multiple IDs, classes, and key-value pairs in a single attribute block:

```markdown
## Results {#results .data .highlight lang="en"}
```
