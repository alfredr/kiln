---
title: "Selectors"
---

Selectors are patterns that match nodes in the document tree. They control which elements receive cascading properties from `@hint` directives and configuration hints. The syntax draws from CSS, adapted for kiln's document model.

## Simple Selectors

### Kind

A bare name matches all nodes of that kind. Both kebab-case and short aliases are accepted.

```markdown
{@hint(figure, series="figs")}
{@hint(code-block, theme="dark")}
{@hint(h, numbering=true)}
```

### Class

A dot-prefixed name matches nodes with that CSS class.

```markdown
{@hint(.wide, layout="full")}
{@hint(.center, text-align="center")}
```

### ID

A hash-prefixed name matches the node with that explicit ID. Auto-generated IDs are never matched.

```markdown
{@hint(#intro, highlight=true)}
```

### Attribute

Square brackets match on node attributes. Structural fields (like `level`, `lang`, `format`) and user-set properties are both accessible.

```markdown
{@hint([lang="rust"], theme="dark")}
{@hint([level=2], numbering=true)}
{@hint([preview], border=true)}
```

`[preview]` without a value tests that the attribute exists (is set to any value).

### This

The keyword `this` matches the element the hint is attached to. Use it inside brace attributes.

```markdown
## Chapter One {#ch1 {@hint(this, numbering=true)}}
```

## Kind Name Registry

Both descriptive kebab-case names and short aliases resolve to the same canonical kind.

| Kind | Kebab-case | Alias |
| --- | --- | --- |
| Section | `section` | `sec` |
| Heading | `heading` | `h` |
| Paragraph | `paragraph` | `p` |
| BlockQuote | `block-quote` | `bq` |
| List | `list` | `list` |
| ListItem | `list-item` | `li` |
| Table | `table` | `table` |
| TableRow | `table-row` | `tr` |
| TableCell | `table-cell` | `td` |
| ThematicBreak | `thematic-break` | `hr` |
| Figure | `figure` | `fig` |
| Caption | `caption` | `cap` |
| CodeBlock | `code-block` | `code` |
| RawBlock | `raw-block` | `raw` |
| Math | `math` | `math` |
| Emphasis | `emphasis` | `em` |
| Strong | `strong` | `strong` |
| Strikethrough | `strikethrough` | `del` |
| Link | `link` | `a` |
| Image | `image` | `img` |
| Citation | `citation` | `cite` |
| CrossRef | `cross-ref` | `ref` |

## Kind:ID Shorthand

Combine a kind with a specific ID using colon syntax.

```markdown
{@hint(fig:arch, position="top")}
```

This matches the Figure node whose full ID is `fig:arch`. It is equivalent to `figure#fig:arch`.

A wildcard `*` matches all nodes of that kind, making `fig:*` equivalent to bare `figure`:

```markdown
{@hint(fig:*, series="figs")}
```

## Compound Selectors

Chain simple selectors without spaces to require all parts match (AND).

```markdown
{@hint(figure.wide, layout="full")}
{@hint(heading[level=2], numbering=true)}
{@hint(heading[level=2].numbered, font-weight="bold")}
```

`figure.wide` matches a node that is both kind Figure and has class `wide`.

## Combinators

### Child: `>`

Matches a node that is a direct child of the left-hand node.

```markdown
{@hint(figure > caption, font-style="italic")}
```

This applies `font-style="italic"` to Caption nodes that are direct children of a Figure.

### Descendant: `~>`

Matches a node that is a descendant at any depth of the left-hand node.

```markdown
{@hint(section ~> figure, border=true)}
```

This applies `border=true` to any Figure anywhere inside a Section.

## Expression Algebra

Selectors support boolean AND and OR combinations using three operators with well-defined precedence.

### Operators

| Operator | Meaning | Context |
| --- | --- | --- |
| `&` | Explicit AND | Anywhere |
| `|` | Explicit OR | Anywhere |
| `,` | Context-dependent | AND inside `[...]`, OR outside `[...]` |

Explicit operators (`&`, `|`) bind tighter than `,` because they represent intentional grouping. The comma is the loose, sentence-level separator.

### Precedence

From tightest to loosest:

| Precedence | Operator | Meaning |
| --- | --- | --- |
| 1 (tightest) | Juxtaposition | Compound AND: `figure.wide[x]` |
| 2 | `>`, `~>` | Combinators |
| 3 | `&` | Explicit AND |
| 4 | `|` | Explicit OR |
| 5 (loosest) | `,` | AND inside `[...]`, OR outside `[...]` |

### Inside `[...]`

Comma means AND between attribute predicates.

| Expression | Grouping | Result |
| --- | --- | --- |
| `[a, b]` | `a , b` | a AND b |
| `[a & b]` | `a & b` | a AND b |
| `[a | b]` | `a | b` | a OR b |
| `[a, b | c]` | `a , (b | c)` | a AND (b OR c) |
| `[a | b, c]` | `(a | b) , c` | (a OR b) AND c |
| `[a & b | c]` | `(a & b) | c` | (a AND b) OR c |
| `[a & b, c | d]` | `(a & b) , (c | d)` | (a AND b) AND (c OR d) |

Chaining separate bracket blocks is also AND:

```markdown
{@hint([lang="rust"][preview], theme="dark")}
```

This is equivalent to `[lang="rust", preview]`.

### Outside `[...]`

Comma means OR between selector expressions.

| Expression | Grouping | Result |
| --- | --- | --- |
| `A, B` | `A , B` | A OR B |
| `A & B, C` | `(A & B) , C` | (A AND B) OR C |
| `A, B & C` | `A , (B & C)` | A OR (B AND C) |
| `A & B | C` | `(A & B) | C` | (A AND B) OR C |

### Tuple Distribution

When an attribute value is a tuple, the equality test distributes into an OR:

```markdown
{@hint([lang=("rust", "python")], theme="dark")}
```

`lang=("rust", "python")` expands to `lang="rust" | lang="python"`. Combined with other predicates via comma (AND inside brackets):

```markdown
{@hint([lang=("rust", "python"), preview], theme="dark")}
```

This means: `(lang="rust" OR lang="python") AND preview`.

## Structural Attributes

Certain node fields are exposed as virtual attributes for selector matching.

| Kind | Attribute | Values |
| --- | --- | --- |
| Heading, Section | `level` | 1, 2, 3, ... |
| CodeBlock | `lang` | "rust", "python", "html", ... |
| RawBlock | `format` | "typst", "html", "mermaid", ... |
| List | `ordered` | "true", "false" |
| Math | `display` | "true", "false" |

User-set properties from brace attributes (`{width="80%"}`) are also matchable.

## Multiple Properties

A single hint can set multiple properties. Separate them with commas after the selector:

```markdown
{@hint(figure, layout="full", position="top")}
```

The boundary between selector and properties is the first top-level `key=value` token. Everything before it is the selector; everything after is properties.

## Full Examples

Apply a theme to all Rust code blocks:

```markdown
{@hint(code-block[lang="rust"], theme="dark", line-numbers=true)}
```

Center all figures with class `hero`:

```markdown
{@hint(figure.hero, text-align="center", width="100%")}
```

Style captions inside figures:

```markdown
{@hint(figure > caption, font-style="italic", color="gray")}
```

Match multiple kinds:

```markdown
{@hint(figure, code-block, border=true)}
```

Target code blocks in Rust or Python:

```markdown
{@hint(code-block[lang=("rust", "python")], theme="dark")}
```
