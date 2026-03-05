---
title: "Cross-References & Citations"
---

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
