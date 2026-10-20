---
title: "Cross-References & Citations"
---

## Cross-References {#sec:cross-refs}

Reference any element in the document by its ID using the `@kind:label` syntax:

```markdown
As shown in @fig:architecture, the pipeline has three stages.

See @sec:intro for background on the problem.

Applying @eq:energy to this scenario yields the expected result.
```

The reference links to the element whose `id` matches the full `kind:label` string. For example, `@fig:architecture` links to the element with `id="fig:architecture"`.

### Reference Termination

A cross-reference is terminated at the first character that is not alphanumeric, a hyphen, an underscore, or a colon. Colons are allowed because the `kind:label` convention uses them. This means you can use references naturally within prose:

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

## Citations {#sec:citations}

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

### Bibliography {#sec:bibliography}

To resolve citations at build time instead, name a BibTeX or BibLaTeX file and a citation style in `kiln.toml`:

```toml
[bibliography]
file = "data/references.bib"
style = "ieee"
```

The style is a name from [Hayagriva](https://github.com/typst/hayagriva)'s archive of CSL styles, such as `ieee`, `apa` or `chicago-author-date`; it defaults to `ieee`. Kiln then formats each citation in the style and links it to the work it cites. Citations written together, like `[@smith2024; @jones2025]`, become one citation:

```html
<cite data-key="smith2024"><a href="#ref-smith2024">[1]</a>, <a href="#ref-jones2025">[2]</a></cite>
```

Templates get the works the page cites as a `bibliography` variable, a `<ul class="references">` whose items have ids such as `ref-smith2024`. It is empty on a page without citations:

```html
{% if bibliography %}
<h2>References</h2>
{{ bibliography | safe }}
{% endif %}
```

A key the file does not have is reported during the build and rendered as before.
