---
title: "Directives"
---

Directives use the `{@command(args)}` syntax to declare document-level configuration. Place a directive on its own line.

## Def / Use {#sec:def-use}

Group declarations into a named block with `@def`, then activate them with `@use`. This is the foundation of kiln's numbering system — the built-in prelude uses `@def`/`@use` to define all standard numbering.

```
@def(numbering) {
  @var(fig, 0);
  @var(index, [0], format=["I","1"]);
  @var(val, 0);

  @on(enter, section > section) {
    @pop(index, val); @inc(val); @push(index, val); @push(index, 0);
  }
  @on(exit, section > section) { @pop(index); }
  @on(enter, heading) {
    @pop(index, val); @store(number, index); @push(index, val);
  }
  @on(enter, figure) { @inc(fig); @store(number, fig); }
  @content(heading, "{number}. ");
}

@use(numbering);
```

`@def` registers the block by name without executing it. `@use` expands the block at the call site. This lets prelude files define reusable numbering schemes that documents opt into.

### Parameterized Def / Use

`@def` can declare parameters with defaults, and `@use` can override them. Use `@arg(name)` inside the body to reference a parameter:

```
@def(numbering, format=["I","1"], content="{number}. ") {
  @var(index, [0], format=@arg(format));
  @content(heading, @arg(content));
  # ... other declarations ...
}

@use(numbering);                                   # all defaults
@use(numbering, format=["1","1"]);                 # override format only
@use(numbering, format=["1","a"], content="§{number} ");  # override both
```

When `@use` has overrides, the body is re-expanded with `@arg(name)` replaced by the override value (or the default if not overridden). When there are no overrides and no params, the pre-parsed body is used directly.

Parameters are untyped text — `@arg(name)` is replaced with the value string, and the surrounding context determines the type. An array value like `["1","a"]` stays an array; a string value like `"{number}. "` stays a string.

## Init {#sec:init}

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

### Using Init in Brace Attributes

You can place an init directive inside a brace attribute block to scope it to the current element:

```markdown
## Chapter One {#ch1 {@init(this, numbering=true)}}
```

## Var {#sec:var}

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

## On {#sec:on}

Bind actions to nodes matching a selector. Actions fire in declaration order during a document-order tree walk:

```markdown
{@on(figure, @inc(fig), @store(number, fig))}
{@on(heading, @inc(sec), @store(number, sec), @reset(fig))}
{@on(heading, @prepend(@text("{number}. ")))}
```

The first argument is a CSS-like selector (kind names, classes, IDs, attribute matchers, combinators). All subsequent `@`-prefixed arguments are actions.

An optional event phase can precede the selector. `enter` (the default) fires before children are processed; `exit` fires after; `click` fires on user interaction in interactive/browser mode:

```markdown
{@on(enter, section > section, @inc(depth))}
{@on(exit, section > section, @dec(depth))}
{@on(click, heading, @inc(count), @store(number, count))}
```

**Available actions:**

| Action | Effect |
| --- | --- |
| `@inc(var)` | Increment a numeric variable by 1. |
| `@dec(var)` | Decrement a numeric variable by 1. |
| `@reset(var)` | Reset variable to its initial value. |
| `@reset(var, value)` | Reset variable to a specific value. |
| `@pop(list)` | Pop the last element from a list (discard). |
| `@pop(list, var)` | Pop the last element from a list into a variable. |
| `@push(list, var)` | Push a variable's value onto a list. |
| `@push(list, 0)` | Push a literal value onto a list. |
| `@store(prop, var)` | Format the variable using its declared format tokens and store the result as a node property. |
| `@set(prop, value)` | Set a node property to a literal value. |
| `@prepend(@text("tpl"))` | Insert a text node before existing children. |
| `@append(@text("tpl"))` | Insert a text node after existing children. |

Text templates support `{key}` interpolation from node properties. For example, `{number}` reads the node's `number` property. Missing keys interpolate as empty strings.

## Content {#sec:content}

Shorthand for `@on(SELECTOR, @prepend(@text(TEMPLATE)))` — prepend formatted text to matching nodes:

```markdown
{@content(heading, "{number}. ")}
```

This is equivalent to:

```markdown
{@on(heading, @prepend(@text("{number}. ")))}
```

## Composing a Complete Numbering Scheme {#sec:composing}

Numbering is built from `@var`, `@on`, `@init`, and `@content` primitives. The `@def`/`@use` mechanism groups them into reusable blocks. Here are several complete examples.

**Flat figure numbering with cross-reference labels:**

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

**Hierarchical section numbering with cross-references:**

The built-in prelude provides this automatically — headings are numbered and `@content` prepends the visible number:

```markdown
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

**Three-level numbering with a custom prelude:**

Create a `.kiln` prelude that overrides the format parameter to add a third level:

```
# Copy the @def from the built-in prelude, then override at @use:
@use(numbering, format=["I","1","a"]);
@init(heading, label="Section {n}");
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

## Prelude {#sec:prelude}

kiln ships a built-in prelude that defines hierarchical section numbering and flat figure numbering. The prelude is automatically loaded for every document. It defines a parameterized `@def(numbering)` block with two customizable parameters:

- `format` — number format tokens per depth level (default: `["I","1"]` for Roman/Arabic)
- `content` — text template prepended to headings (default: `"{number}. "`)

The prelude also sets default cross-reference labels via `@init`:

```
@init(figure, label="Figure {n}");
@init(heading, label="Section {n}");
```

This means headings are automatically numbered and `@fig:name`/`@sec:name` cross-references work out of the box:

```markdown
# Introduction {#sec:intro}

## Background {#sec:bg}

See @sec:intro for context.
```

Result: headings render as "I. Introduction", "I.1. Background", and the cross-reference resolves to "Section I".

### `.kiln` File Format

The prelude is a `.kiln` file with `@def`/`@use` blocks, inline directives, and `#` for comments. A custom prelude can override the numbering parameters at the `@use` site without duplicating the full `@def` body:

```
# My custom prelude — Arabic numbering with short labels
@def(numbering, format=["I","1"], content="{number}. ") {
  # ... same body as built-in (with @arg references) ...
}
@use(numbering, format=["1","a"]);
@init(figure, label="Fig. {n}");
@init(heading, label="§{n}");
```

### Overriding the Prelude

To use a custom prelude instead of the built-in one, set the `prelude` field in `kiln.toml`:

```toml
[build]
prelude = "my-prelude.kiln"
```

The path is relative to the project root. The custom file completely replaces the built-in prelude.
