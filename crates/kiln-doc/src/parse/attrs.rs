use crate::ast::{
    Action, Attrs, Bind, Declaration, Event, Init, Node, NodeKind, OnDirective, Value, VarDecl,
};

use super::inline::find_matching_delim;

/// Extract `{...}` at the end of a line, returning (text_before, Some(brace_content)).
pub(super) fn extract_trailing_brace_attrs(text: &str) -> (&str, Option<String>) {
    let trimmed = text.trim_end();
    if !trimmed.ends_with('}') {
        return (text, None);
    }
    let start = find_matching_open_brace(trimmed);
    if let Some(s) = start {
        let before = trimmed[..s].trim_end();
        let brace_content = &trimmed[s + 1..trimmed.len() - 1];
        (before, Some(brace_content.to_string()))
    } else {
        (text, None)
    }
}

/// Extract a `=format` token from brace attributes (e.g. `{=html}` or `{=typst #id .class}`).
/// Also handles the colon shorthand `{=typst:#id}` — splits at the first `:` to separate
/// format from trailing attrs.
/// Returns `(Some(format), remaining_attrs)` if found, `(None, original_attrs)` otherwise.
pub(super) fn extract_raw_format(attrs: Option<&str>) -> (Option<String>, Option<String>) {
    let attrs = match attrs {
        Some(a) => a.trim(),
        None => return (None, None),
    };

    let mut result_format = None;
    let mut remaining_parts = Vec::new();

    for token in attrs.split_whitespace() {
        if result_format.is_none() && token.starts_with('=') && token.len() > 1 {
            let (fmt, colon_rest) = split_format_token(&token[1..]);
            result_format = Some(fmt);
            remaining_parts.extend(colon_rest);
        } else {
            remaining_parts.push(token.to_string());
        }
    }

    let remaining = if remaining_parts.is_empty() {
        None
    } else {
        Some(remaining_parts.join(" "))
    };

    (result_format, remaining)
}

/// Split a format token at the first `:` for the colon shorthand.
/// `"typst:#id"` → `("typst", Some("#id"))`, `"html"` → `("html", None)`.
fn split_format_token(value: &str) -> (String, Option<String>) {
    let Some(colon) = value.find(':') else {
        return (value.to_string(), None);
    };
    let format = value[..colon].to_string();
    let after = &value[colon + 1..];
    let rest = if after.is_empty() {
        None
    } else {
        Some(after.to_string())
    };
    (format, rest)
}

/// Extract a language token from brace attributes.
/// The language is the first bare word that isn't `#id`, `.class`, `@directive`,
/// `=format`, or `key=value`.
/// Returns `(Some(lang), remaining_attrs)` or `(None, original_attrs)`.
pub(super) fn extract_lang(attrs: Option<&str>) -> (Option<String>, Option<String>) {
    let attrs = match attrs {
        Some(a) if !a.trim().is_empty() => a.trim(),
        _ => return (None, None),
    };

    let mut lang = None;
    let mut remaining = Vec::new();

    for token in attrs.split_whitespace() {
        if lang.is_none()
            && !token.starts_with('#')
            && !token.starts_with('.')
            && !token.starts_with('@')
            && !token.starts_with('=')
            && !token.contains('=')
        {
            lang = Some(token.to_string());
        } else {
            remaining.push(token);
        }
    }

    let rest = if remaining.is_empty() {
        None
    } else {
        Some(remaining.join(" "))
    };

    (lang, rest)
}

pub(super) fn apply_brace_attrs(attrs: &mut Attrs, content: &str) {
    let content = content.trim();
    let mut pos = 0;
    let chars: Vec<char> = content.chars().collect();

    while pos < chars.len() {
        if chars[pos].is_whitespace() {
            pos += 1;
            continue;
        }
        match chars[pos] {
            '#' | '.' => {
                pos = scan_id_or_class(chars[pos], &chars, pos + 1, attrs);
            }
            '@' => {
                pos = parse_attr_directive(&chars, pos, attrs);
            }
            _ => {
                pos = parse_attr_key_value(&chars, pos, attrs);
            }
        }
    }
}

/// Check the last child for trailing brace attrs and apply to parent or preceding node.
pub(super) fn maybe_extract_trailing_attrs_from_children(children: &mut Vec<Node>) {
    let Some((attr_str, before)) = extract_trailing_brace_from_last_child(children) else {
        return;
    };
    if before.is_empty() {
        children.pop();
        if let Some(prev) = children.last_mut() {
            apply_brace_attrs(&mut prev.attrs, &attr_str);
        }
    } else {
        *children.last_mut().expect("children non-empty") = Node::text(before);
        let len = children.len();
        if len >= 2 {
            let (head, _tail) = children.split_at_mut(len - 1);
            apply_brace_attrs(
                &mut head.last_mut().expect("head non-empty").attrs,
                &attr_str,
            );
        }
    }
}

/// Parse a standalone `@directive(...)` into a node carrying declarations/inits.
pub(super) fn parse_directive_to_node(content: &str) -> Option<Node> {
    let mut node = Node::new(NodeKind::Custom {
        content_type: "directive".to_string(),
    });
    // content looks like: @var(...) or @init(...)
    let content = content.trim();
    if !content.starts_with('@') {
        return None;
    }
    let rest = &content[1..];
    let paren = rest.find('(')?;
    let cmd = &rest[..paren];
    let close = rest.rfind(')')?;
    let args = &rest[paren + 1..close];
    apply_directive(&mut node.attrs, cmd, args);
    Some(node)
}

// ── Internal helpers ──

fn scan_token(chars: &[char], pos: usize) -> (String, usize) {
    let start = pos;
    let mut p = pos;
    while p < chars.len() && !chars[p].is_whitespace() {
        p += 1;
    }
    (chars[start..p].iter().collect(), p)
}

fn parse_attr_directive(chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let mut p = pos + 1; // skip '@'
    while p < chars.len() && (chars[p].is_alphanumeric() || chars[p] == '_') {
        p += 1;
    }
    let cmd: String = chars[pos + 1..p].iter().collect();
    if p < chars.len() && chars[p] == '(' {
        let paren_start = p;
        if let Some(close) = find_matching_delim(chars, p, '(', ')') {
            let args: String = chars[paren_start + 1..close].iter().collect();
            apply_directive(attrs, &cmd, &args);
            return close + 1;
        }
    }
    p
}

fn parse_attr_key_value(chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let key_start = pos;
    let mut p = pos;
    while p < chars.len() && chars[p] != '=' && !chars[p].is_whitespace() {
        p += 1;
    }
    let key: String = chars[key_start..p].iter().collect();
    if key.is_empty() {
        return p + 1;
    }
    if p >= chars.len() || chars[p] != '=' {
        // Bare token without value — treat as boolean true.
        attrs.properties.insert(key, Value::Bool(true));
        return p;
    }
    p += 1; // skip =
    let (value, new_pos) = parse_attr_value(chars, p);
    attrs.properties.insert(key, Value::Text(value));
    new_pos
}

fn parse_attr_value(chars: &[char], pos: usize) -> (String, usize) {
    if pos < chars.len() && chars[pos] == '"' {
        let mut p = pos + 1;
        let val_start = p;
        while p < chars.len() && chars[p] != '"' {
            p += 1;
        }
        let v: String = chars[val_start..p].iter().collect();
        if p < chars.len() {
            p += 1; // skip closing quote
        }
        (v, p)
    } else {
        let (v, p) = scan_token(chars, pos);
        (v, p)
    }
}

fn scan_id_or_class(sigil: char, chars: &[char], pos: usize, attrs: &mut Attrs) -> usize {
    let (token, new_pos) = scan_token(chars, pos);
    if sigil == '#' {
        attrs.id = token;
        attrs.auto_id = false;
    } else {
        attrs.classes.push(token);
    }
    new_pos
}

/// Parse a `.kiln` prelude file into declarations and inits.
///
/// Supports both inline syntax (`@var(fig, 0)`) and block syntax:
/// ```text
/// @def(name) {
///   @var(x, 0);
///   @on(enter, section) { @pop(x); }
/// }
/// @use(name);
/// ```
/// Lines starting with `#` are comments.
pub fn parse_prelude(text: &str) -> (Vec<Declaration>, Vec<Init>) {
    let mut attrs = Attrs::default();
    // Strip comments, join into single string for block parsing
    let stripped: String = text
        .lines()
        .map(|l| {
            let trimmed = l.trim();
            if trimmed.starts_with('#') { "" } else { trimmed }
        })
        .collect::<Vec<_>>()
        .join(" ");
    parse_prelude_statements(&stripped, &mut attrs);
    (attrs.declarations, attrs.inits)
}

/// Parse a sequence of statements from a prelude string.
/// Statements are `@cmd(args);` or `@cmd(args) { body }`.
fn parse_prelude_statements(input: &str, attrs: &mut Attrs) {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    while pos < chars.len() {
        pos = parse_one_prelude_statement(&chars, pos, attrs);
    }
}

/// Parse a single `@cmd(args)` or `@cmd(args) { body }` statement,
/// returning the updated position.
fn parse_one_prelude_statement(chars: &[char], mut pos: usize, attrs: &mut Attrs) -> usize {
    // Skip whitespace
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos >= chars.len() {
        return pos;
    }
    // Expect @
    if chars[pos] != '@' {
        return pos + 1;
    }
    pos += 1; // skip @
    // Read command name
    let cmd_start = pos;
    while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
        pos += 1;
    }
    let cmd: String = chars[cmd_start..pos].iter().collect();
    // Skip whitespace
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos >= chars.len() || chars[pos] != '(' {
        return pos;
    }
    // Find matching close paren
    let Some(close_paren) = find_matching_delim(chars, pos, '(', ')') else {
        return chars.len();
    };
    let args: String = chars[pos + 1..close_paren].iter().collect();
    pos = close_paren + 1;

    // Skip whitespace
    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }

    // Check for block body { ... }
    if pos < chars.len() && chars[pos] == '{' {
        let Some(close_brace) = find_matching_delim(chars, pos, '{', '}') else {
            return chars.len();
        };
        let body: String = chars[pos + 1..close_brace].iter().collect();
        pos = close_brace + 1;
        // Skip optional trailing semicolon
        while pos < chars.len() && (chars[pos].is_whitespace() || chars[pos] == ';') {
            pos += 1;
        }
        apply_block_directive(attrs, &cmd, &args, &body);
    } else {
        // Inline statement — skip optional trailing semicolon
        if pos < chars.len() && chars[pos] == ';' {
            pos += 1;
        }
        apply_directive(attrs, &cmd, &args);
    }
    pos
}

/// Handle a block directive: `@cmd(args) { body }`.
fn apply_block_directive(attrs: &mut Attrs, cmd: &str, args: &str, body: &str) {
    match cmd {
        "def" => {
            let parts = split_directive_args(args);
            if parts.is_empty() {
                return;
            }
            let name = parts[0].0.clone();
            let params: Vec<(String, String)> = parts[1..]
                .iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mut inner = Attrs::default();
            parse_prelude_statements(body, &mut inner);
            attrs.declarations.push(Declaration::Def {
                name,
                params,
                body: inner.declarations,
                inits: inner.inits,
            });
        }
        "on" => {
            // Parse event + selector from args, actions from body
            if let Some(on) = parse_on_block_directive(args, body) {
                attrs.declarations.push(Declaration::OnDirective(on));
            }
        }
        _ => {}
    }
}

/// Parse `@on(enter, selector) { @action1(...); @action2(...); }`.
fn parse_on_block_directive(args: &str, body: &str) -> Option<OnDirective> {
    let parts = split_top_level_commas(args);
    if parts.is_empty() {
        return None;
    }

    // Check for explicit event keyword
    let (event, selector_parts) = match parts[0].trim() {
        "enter" => (Event::Enter, &parts[1..]),
        "exit" => (Event::Exit, &parts[1..]),
        "click" => (Event::Click, &parts[1..]),
        _ => (Event::Enter, &parts[..]),
    };

    let selector_str = selector_parts
        .iter()
        .map(|s| s.trim())
        .collect::<Vec<_>>()
        .join(",");
    let selector = crate::selector::parse_selector(&selector_str)?;

    // Parse actions from body — semicolon-separated @action(...) calls
    let actions = parse_action_body(body);
    if actions.is_empty() {
        return None;
    }

    Some(OnDirective { event, selector, actions })
}

/// Parse a semicolon-separated block of `@action(...)` calls.
fn parse_action_body(body: &str) -> Vec<Action> {
    body.split(';')
        .filter_map(|stmt| {
            let stmt = stmt.trim();
            if stmt.is_empty() {
                None
            } else {
                parse_action_call(stmt)
            }
        })
        .collect()
}

fn apply_directive(attrs: &mut Attrs, cmd: &str, args: &str) {
    match cmd {
        "init" | "hint" => {
            attrs.inits.extend(parse_init_directive(args));
        }
        "var" => {
            if let Some(var_decl) = parse_var_directive(args) {
                attrs.declarations.push(Declaration::VarDecl(var_decl));
            }
        }
        "on" | "content" => {
            let parsed = if cmd == "on" {
                parse_on_directive(args)
            } else {
                parse_content_directive(args)
            };
            if let Some(on) = parsed {
                attrs.declarations.push(Declaration::OnDirective(on));
            }
        }
        "use" => {
            let parts = split_directive_args(args);
            if parts.is_empty() {
                return;
            }
            let name = parts[0].0.clone();
            let overrides: Vec<(String, String)> = parts[1..]
                .iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            if !name.is_empty() {
                attrs.declarations.push(Declaration::Use { name, overrides });
            }
        }
        _ => {}
    }
}

/// Parse `@var(name, value, format=...)`.
///
/// Examples: `@var(fig, 0)`, `@var(sec, [], format=["I","1"])`, `@var(flag, false)`
fn parse_var_directive(args: &str) -> Option<VarDecl> {
    let parts = split_directive_args(args);
    if parts.is_empty() {
        return None;
    }
    let name = parts[0].0.clone();

    // Parse initial value (second positional arg)
    let value = if parts.len() > 1 {
        parse_or_defer(&parts[1].1, parse_initial_value)
    } else {
        Bind::Val(Value::Number(0.0))
    };

    let mut formats = Bind::Val(Vec::new());
    let mut cycle = false;

    for (key, val) in &parts[2..] {
        match key.as_str() {
            "format" | "formats" => formats = parse_or_defer(val, parse_format_list),
            "cycle" => cycle = val == "true",
            _ => {}
        }
    }

    Some(VarDecl {
        name,
        value,
        formats,
        cycle,
    })
}

/// Parse an initial value string: `0` → Number, `[]` → List, `[0]` → List([0]), etc.
pub(crate) fn parse_initial_value(s: &str) -> Value {
    let s = s.trim();
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ if s.starts_with('[') && s.ends_with(']') => {
            let inner = s[1..s.len() - 1].trim();
            Value::List(if inner.is_empty() {
                vec![]
            } else {
                inner.split(',').map(parse_scalar_value).collect()
            })
        }
        _ => parse_scalar_value(s),
    }
}

fn parse_scalar_value(s: &str) -> Value {
    let s = s.trim();
    if let Ok(n) = s.parse::<f64>() {
        Value::Number(n)
    } else {
        Value::Text(s.trim_matches('"').to_string())
    }
}

/// Parse `@on(enter, selector, @action1(...), ...)` or `@on(selector, @action1(...), ...)`.
///
/// If the first token is `enter`, `exit`, or `click`, it sets the event phase;
/// otherwise defaults to `Enter`. Remaining non-`@` tokens form the selector;
/// `@`-prefixed tokens are action calls.
fn parse_on_directive(args: &str) -> Option<OnDirective> {
    let parts = split_top_level_commas(args);
    if parts.is_empty() {
        return None;
    }

    // Check for explicit event keyword as first token
    let (event, rest) = match parts[0].trim() {
        "enter" => (Event::Enter, &parts[1..]),
        "exit" => (Event::Exit, &parts[1..]),
        "click" => (Event::Click, &parts[1..]),
        _ => (Event::Enter, &parts[..]),
    };

    // Find the boundary: first part starting with @
    let action_start = rest.iter().position(|p| p.trim().starts_with('@'));
    let (selector_parts, action_parts) = match action_start {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => return None, // no actions
    };

    let selector_str = selector_parts.join(",");
    let selector = crate::selector::parse_selector(selector_str.trim())?;

    let actions: Vec<Action> = action_parts
        .iter()
        .filter_map(|a| parse_action_call(a.trim()))
        .collect();

    if actions.is_empty() {
        return None;
    }

    Some(OnDirective { event, selector, actions })
}

/// Parse a single `@action(args)` call.
fn parse_action_call(s: &str) -> Option<Action> {
    let s = s.trim();
    if !s.starts_with('@') {
        return None;
    }
    let rest = &s[1..];
    let (cmd, args) = if let Some(paren) = rest.find('(') {
        let close = rest.rfind(')')?;
        (&rest[..paren], rest[paren + 1..close].trim())
    } else {
        (rest, "")
    };

    match cmd {
        "inc" => Some(Action::Increment {
            var: args.trim().to_string(),
        }),
        "dec" => Some(Action::Decrement {
            var: args.trim().to_string(),
        }),
        "pop" => {
            let mut parts = split_directive_args(args).into_iter();
            let list = parts.next()?.0;
            let into = parts.next().map(|(k, _)| k);
            Some(Action::Pop { list, into })
        }
        "push" => {
            let mut parts = parse_args(args, 2)?.into_iter();
            let list = parts.next()?.0;
            let value_str = parts.next()?.0;
            // If it parses as a number, treat as literal; otherwise it's a var reference
            let value = if let Ok(n) = value_str.trim().parse::<f64>() {
                crate::ast::PushValue::Literal(Value::Number(n))
            } else {
                crate::ast::PushValue::Var(value_str.trim().to_string())
            };
            Some(Action::Push { list, value })
        }
        "reset" => {
            let parts = split_directive_args(args);
            let var = parts.first()?.0.clone();
            // Reset to 0 by default; the engine will use the VarDecl's initial value
            let to = if parts.len() > 1 {
                parse_or_defer(&parts[1].1, parse_initial_value)
            } else {
                Bind::Val(Value::Number(0.0))
            };
            Some(Action::Reset { var, to })
        }
        "store" => {
            let parts = parse_args(args, 2)?;
            let property = parts[0].0.clone();
            let var = parts[1].0.clone();
            // Formats come from the VarDecl; at this level we don't know them
            Some(Action::StoreFormatted {
                var,
                property,
                formats: Vec::new(),
                cycle: false,
            })
        }
        "set" => {
            let parts = parse_args(args, 2)?;
            let property = parts[0].0.clone();
            let value = parse_or_defer(&parts[1].1, parse_init_value);
            Some(Action::SetProperty { property, value })
        }
        "prepend" | "content" | "append" => {
            // @content("template") is sugar for @prepend(@text("template"))
            let template = parse_or_defer(&parse_text_arg(args), |s| s.to_string());
            Some(if cmd == "append" {
                Action::Append { template }
            } else {
                Action::Prepend { template }
            })
        }
        _ => None,
    }
}

/// Parse the text argument from `@text("template")` or a bare `"template"` string.
fn parse_text_arg(args: &str) -> String {
    let args = args.trim();
    // Handle @text("...") wrapper
    if args.starts_with("@text(") {
        let inner = args
            .strip_prefix("@text(")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(args);
        return inner.trim().trim_matches('"').to_string();
    }
    args.trim_matches('"').to_string()
}

/// Parse `@content(selector, "template")` — sugar for `@on(selector, @prepend(@text("template")))`.
fn parse_content_directive(args: &str) -> Option<OnDirective> {
    let parts = split_top_level_commas(args);
    if parts.len() < 2 {
        return None;
    }

    let selector_str = parts[0].trim();
    let selector = crate::selector::parse_selector(selector_str)?;
    let raw = parts[1..].join(",").trim().trim_matches('"').to_string();
    let template = parse_or_defer(&raw, |s| s.to_string());

    Some(OnDirective {
        event: Event::Enter,
        selector,
        actions: vec![Action::Prepend { template }],
    })
}

/// Parse a format list like `["I","1","a"]` into a `Vec<String>`.
/// Also accepts a single bare token like `I` → `vec!["I"]`.
/// Check if a string is an `@arg(name)` reference, returning the parameter name.
pub(crate) fn try_parse_arg_ref(s: &str) -> Option<String> {
    let s = s.trim();
    Some(s.strip_prefix("@arg(")?.strip_suffix(')')?.to_string())
}

/// Parse a value string, or return a deferred `@arg` reference.
pub(crate) fn parse_or_defer<T>(s: &str, parse: impl Fn(&str) -> T) -> Bind<T> {
    match try_parse_arg_ref(s) {
        Some(param) => Bind::Arg(param),
        None => Bind::Val(parse(s)),
    }
}

/// Parse a format list like `["I","1","a"]` into a `Vec<String>`.
/// Also accepts a single bare token like `I` → `vec!["I"]`.
pub(crate) fn parse_format_list(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        inner
            .split(',')
            .map(|t| t.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|t| !t.is_empty())
            .collect()
    } else {
        vec![s.to_string()]
    }
}

/// Parse `@init(figure.wide, layout="full", position="top")`.
///
/// The args are split into a selector portion and property assignments.
/// The boundary is the first top-level comma-separated token that looks
/// like a property assignment (`key=value` where key is a bare identifier).
/// Returns one `Init` per property, all sharing the same selector.
fn parse_init_directive(args: &str) -> Vec<Init> {
    let parts = split_top_level_commas(args);
    if parts.is_empty() {
        return vec![];
    }

    // Find the boundary: first token that is a property assignment.
    let boundary = parts.iter().position(|p| is_property_token(p));

    let (selector_str, prop_parts) = match boundary {
        Some(idx) => {
            let sel = parts[..idx].join(",");
            (sel, &parts[idx..])
        }
        None => return vec![],
    };

    let selector = match crate::selector::parse_selector(&selector_str) {
        Some(s) => s,
        None => return vec![],
    };

    let mut inits = Vec::new();
    for prop in prop_parts {
        let prop = prop.trim();
        if let Some(eq) = find_top_level_eq(prop) {
            let key = prop[..eq].trim().to_string();
            let val_str = prop[eq + 1..].trim();
            let value = parse_or_defer(val_str, parse_init_value);
            inits.push(Init {
                selector: selector.clone(),
                property: key,
                value,
            });
        }
    }
    inits
}

/// Check if a comma-separated token is a property assignment (bare identifier followed by `=`).
fn is_property_token(token: &str) -> bool {
    let token = token.trim();
    if token.is_empty()
        || token.starts_with('.')
        || token.starts_with('#')
        || token.starts_with('[')
    {
        return false;
    }
    if token == "this" {
        return false;
    }
    let mut depth = 0i32;
    let mut in_quotes = false;
    for (i, ch) in token.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' | '(' if !in_quotes => depth += 1,
            ']' | ')' if !in_quotes => depth -= 1,
            '=' if !in_quotes && depth == 0 => {
                let before = &token[..i];
                return before
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
            }
            _ => {}
        }
    }
    false
}

/// Parse an init property value string into a `Value`.
pub(crate) fn parse_init_value(s: &str) -> Value {
    let s = s.trim().trim_matches('"');
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(n) = s.parse::<f64>() {
                Value::Number(n)
            } else {
                Value::Text(s.to_string())
            }
        }
    }
}

/// Split init args on top-level commas (not inside `[...]`, `(...)`, or `"`).
fn split_top_level_commas(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_quotes = false;

    for (i, ch) in input.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '[' | '(' if !in_quotes => depth += 1,
            ']' | ')' if !in_quotes => depth -= 1,
            ',' if !in_quotes && depth == 0 => {
                parts.push(input[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(input[start..].to_string());
    parts
}

/// Split directive args like `figs, label="Figure", display=hierarchical`
/// into [(key, value), ...]. Positional args have key == value.
fn scan_delimited_token(chars: &[char], start: usize) -> usize {
    let mut pos = start;
    let mut depth = 0;
    let mut in_quotes = false;
    while pos < chars.len() {
        let ch = chars[pos];
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' | '[' if !in_quotes => depth += 1,
            ')' | ']' if !in_quotes && depth > 0 => depth -= 1,
            ',' if !in_quotes && depth == 0 => break,
            _ => {}
        }
        pos += 1;
    }
    pos
}

fn split_kv(token: &str) -> (String, String) {
    if let Some(eq_pos) = find_top_level_eq(token) {
        let key = token[..eq_pos].trim().to_string();
        let val = token[eq_pos + 1..].trim().trim_matches('"').to_string();
        (key, val)
    } else {
        (token.to_string(), token.to_string())
    }
}

fn split_directive_args(args: &str) -> Vec<(String, String)> {
    let mut pos = 0;
    let chars: Vec<char> = args.chars().collect();
    std::iter::from_fn(|| {
        while pos < chars.len() && (chars[pos].is_whitespace() || chars[pos] == ',') {
            pos += 1;
        }
        if pos >= chars.len() {
            return None;
        }
        let start = pos;
        pos = scan_delimited_token(&chars, start);
        let token: String = chars[start..pos].iter().collect();
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        Some(split_kv(token))
    })
    .collect()
}

fn parse_args(args: &str, min: usize) -> Option<Vec<(String, String)>> {
    let parts = split_directive_args(args);
    (parts.len() >= min).then_some(parts)
}

fn find_top_level_eq(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_quotes = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '(' | '[' if !in_quotes => depth += 1,
            ')' | ']' if !in_quotes => depth -= 1,
            '=' if !in_quotes && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn extract_trailing_brace_from_last_child(children: &[Node]) -> Option<(String, String)> {
    let last = children.last()?;
    let NodeKind::Text(ref text) = last.kind else {
        return None;
    };
    let trimmed = text.trim_end();
    let brace_start = find_matching_open_brace(trimmed)?;
    let attr_str = trimmed[brace_start + 1..trimmed.len() - 1].to_string();
    let before = trimmed[..brace_start].trim_end().to_string();
    Some((attr_str, before))
}

fn find_matching_open_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.last() != Some(&b'}') {
        return None;
    }
    let mut depth = 0i32;
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'}' {
            depth += 1;
        } else if bytes[i] == b'{' {
            depth -= 1;
        }
        if depth == 0 {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_trailing_brace_attrs ──

    #[test]
    fn trailing_brace_basic() {
        let (before, attrs) = extract_trailing_brace_attrs("text {#id}");
        assert_eq!(before, "text");
        assert_eq!(attrs.as_deref(), Some("#id"));
    }

    #[test]
    fn trailing_brace_none_when_no_brace() {
        let (before, attrs) = extract_trailing_brace_attrs("no braces here");
        assert_eq!(before, "no braces here");
        assert!(attrs.is_none());
    }

    #[test]
    fn trailing_brace_unmatched() {
        let (_, attrs) = extract_trailing_brace_attrs("unmatched }");
        // Unmatched } should still try to find matching {
        // Since there's no {, should return None
        assert!(attrs.is_none());
    }

    #[test]
    fn trailing_brace_nested() {
        let (before, attrs) = extract_trailing_brace_attrs("text {outer {inner}}");
        assert_eq!(before, "text");
        assert_eq!(attrs.as_deref(), Some("outer {inner}"));
    }

    // ── extract_raw_format ──

    #[test]
    fn raw_format_none_input() {
        let (fmt, rest) = extract_raw_format(None);
        assert!(fmt.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn raw_format_html() {
        let (fmt, rest) = extract_raw_format(Some("=html"));
        assert_eq!(fmt.as_deref(), Some("html"));
        assert!(rest.is_none());
    }

    #[test]
    fn raw_format_with_remaining() {
        let (fmt, rest) = extract_raw_format(Some("=html #id .class"));
        assert_eq!(fmt.as_deref(), Some("html"));
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn raw_format_no_equals() {
        let (fmt, rest) = extract_raw_format(Some("rust #id"));
        assert!(fmt.is_none());
        assert_eq!(rest.as_deref(), Some("rust #id"));
    }

    #[test]
    fn raw_format_equals_only() {
        // "=" alone — no format extracted (len == 1, not > 1)
        let (fmt, rest) = extract_raw_format(Some("="));
        assert!(fmt.is_none());
        assert_eq!(rest.as_deref(), Some("="));
    }

    #[test]
    fn raw_format_colon_shorthand() {
        let (fmt, rest) = extract_raw_format(Some("=typst:#id"));
        assert_eq!(fmt.as_deref(), Some("typst"));
        assert_eq!(rest.as_deref(), Some("#id"));
    }

    // ── split_format_token ──

    #[test]
    fn split_format_no_colon() {
        let (fmt, rest) = split_format_token("html");
        assert_eq!(fmt, "html");
        assert!(rest.is_none());
    }

    #[test]
    fn split_format_colon_with_rest() {
        let (fmt, rest) = split_format_token("typst:#id");
        assert_eq!(fmt, "typst");
        assert_eq!(rest.as_deref(), Some("#id"));
    }

    #[test]
    fn split_format_trailing_colon() {
        let (fmt, rest) = split_format_token("typst:");
        assert_eq!(fmt, "typst");
        assert!(rest.is_none());
    }

    // ── extract_lang ──

    #[test]
    fn extract_lang_bare_word() {
        let (lang, rest) = extract_lang(Some("rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_with_attrs() {
        let (lang, rest) = extract_lang(Some("rust #id .class"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn extract_lang_no_bare_word() {
        let (lang, rest) = extract_lang(Some("#id .class"));
        assert!(lang.is_none());
        assert_eq!(rest.as_deref(), Some("#id .class"));
    }

    #[test]
    fn extract_lang_none() {
        let (lang, rest) = extract_lang(None);
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_empty() {
        let (lang, rest) = extract_lang(Some(""));
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_whitespace_only() {
        let (lang, rest) = extract_lang(Some("   "));
        assert!(lang.is_none());
        assert!(rest.is_none());
    }

    #[test]
    fn extract_lang_key_value_skipped() {
        let (lang, rest) = extract_lang(Some("key=value rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("key=value"));
    }

    #[test]
    fn extract_lang_format_skipped() {
        let (lang, rest) = extract_lang(Some("=html rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("=html"));
    }

    #[test]
    fn extract_lang_directive_skipped() {
        let (lang, rest) = extract_lang(Some("@hint rust"));
        assert_eq!(lang.as_deref(), Some("rust"));
        assert_eq!(rest.as_deref(), Some("@hint"));
    }

    // ── apply_brace_attrs ──

    #[test]
    fn brace_attrs_id() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "#my-id");
        assert_eq!(attrs.id, "my-id");
        assert!(!attrs.auto_id);
    }

    #[test]
    fn brace_attrs_class() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, ".wide");
        assert_eq!(attrs.classes, vec!["wide"]);
    }

    #[test]
    fn brace_attrs_key_value() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "width=\"80%\"");
        assert!(matches!(attrs.properties.get("width"), Some(Value::Text(s)) if s == "80%"));
    }

    #[test]
    fn brace_attrs_bare_boolean() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "nowrap");
        assert!(matches!(attrs.properties.get("nowrap"), Some(Value::Bool(true))));
    }

    #[test]
    fn brace_attrs_directive() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "@hint(figure, series=figs)");
        assert!(!attrs.inits.is_empty());
    }

    #[test]
    fn brace_attrs_multiple() {
        let mut attrs = Attrs::default();
        apply_brace_attrs(&mut attrs, "#my-id .wide .dark nowrap");
        assert_eq!(attrs.id, "my-id");
        assert!(!attrs.auto_id);
        assert!(attrs.classes.contains(&"wide".to_string()));
        assert!(attrs.classes.contains(&"dark".to_string()));
        assert!(matches!(attrs.properties.get("nowrap"), Some(Value::Bool(true))));
    }

    #[test]
    fn brace_attrs_whitespace_only() {
        let mut attrs = Attrs::default();
        let orig_id = attrs.id.clone();
        apply_brace_attrs(&mut attrs, "   ");
        assert_eq!(attrs.id, orig_id);
    }

    // ── parse_directive_to_node ──

    #[test]
    fn directive_to_node_hint() {
        let node = parse_directive_to_node("@hint(figure, series=figs)");
        assert!(node.is_some());
        let n = node.unwrap();
        assert!(!n.attrs.inits.is_empty());
    }

    #[test]
    fn directive_to_node_not_directive() {
        let node = parse_directive_to_node("not a directive");
        assert!(node.is_none());
    }

    #[test]
    fn directive_to_node_no_parens() {
        let node = parse_directive_to_node("@var");
        assert!(node.is_none());
    }

    #[test]
    fn directive_to_node_unclosed_paren() {
        let node = parse_directive_to_node("@var(fig, 0");
        assert!(node.is_none());
    }

    // ── maybe_extract_trailing_attrs_from_children ──

    #[test]
    fn trailing_attrs_applied_to_previous_node() {
        let mut children = vec![
            Node::emphasis(vec![Node::text("em")]),
            Node::text(" {.highlight}"),
        ];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // The {.highlight} should be applied to the emphasis node
        assert_eq!(children.len(), 1);
        assert!(children[0].attrs.classes.contains(&"highlight".to_string()));
    }

    #[test]
    fn trailing_attrs_no_brace() {
        let mut children = vec![Node::text("no braces")];
        maybe_extract_trailing_attrs_from_children(&mut children);
        assert_eq!(children.len(), 1);
    }

    #[test]
    fn trailing_attrs_text_before_brace() {
        let mut children = vec![
            Node::emphasis(vec![Node::text("em")]),
            Node::text("text {.cls}"),
        ];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // Should split: keep "text" node and apply .cls to emphasis
        assert_eq!(children.len(), 2);
        assert!(children[0].attrs.classes.contains(&"cls".to_string()));
    }

    #[test]
    fn trailing_attrs_single_child_pure_brace() {
        let mut children = vec![Node::text("{.cls}")];
        maybe_extract_trailing_attrs_from_children(&mut children);
        // Single child with only braces — pop it, but no previous node to apply to
        assert!(children.is_empty());
    }

    // ── parse_init_value ──

    #[test]
    fn hint_value_true() {
        assert!(matches!(parse_init_value("true"), Value::Bool(true)));
    }

    #[test]
    fn hint_value_false() {
        assert!(matches!(parse_init_value("false"), Value::Bool(false)));
    }

    #[test]
    fn hint_value_number() {
        assert!(matches!(parse_init_value("42"), Value::Number(n) if n == 42.0));
    }

    #[test]
    fn hint_value_float() {
        assert!(matches!(parse_init_value("2.5"), Value::Number(n) if (n - 2.5).abs() < 0.01));
    }

    #[test]
    fn hint_value_text() {
        assert!(matches!(parse_init_value("hello"), Value::Text(ref s) if s == "hello"));
    }

    #[test]
    fn hint_value_quoted() {
        assert!(matches!(parse_init_value("\"hello\""), Value::Text(ref s) if s == "hello"));
    }

    // ── is_property_token ──

    #[test]
    fn property_token_key_value() {
        assert!(is_property_token("key=value"));
    }

    #[test]
    fn property_token_bare_word_not_property() {
        assert!(!is_property_token("figure"));
    }

    #[test]
    fn property_token_class_not_property() {
        assert!(!is_property_token(".wide"));
    }

    #[test]
    fn property_token_id_not_property() {
        assert!(!is_property_token("#my-id"));
    }

    #[test]
    fn property_token_bracket_not_property() {
        assert!(!is_property_token("[attr]"));
    }

    #[test]
    fn property_token_this_not_property() {
        assert!(!is_property_token("this"));
    }

    #[test]
    fn property_token_empty_not_property() {
        assert!(!is_property_token(""));
    }

    // ── split_top_level_commas ──

    #[test]
    fn split_commas_simple() {
        let parts = split_top_level_commas("a, b, c");
        assert_eq!(parts, vec!["a", " b", " c"]);
    }

    #[test]
    fn split_commas_nested_brackets() {
        let parts = split_top_level_commas("sel[a,b], key=val");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "sel[a,b]");
    }

    #[test]
    fn split_commas_quoted() {
        let parts = split_top_level_commas("key=\"a,b\", other=c");
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn split_commas_no_comma() {
        let parts = split_top_level_commas("single");
        assert_eq!(parts, vec!["single"]);
    }

    // ── find_top_level_eq ──

    #[test]
    fn top_level_eq_simple() {
        assert_eq!(find_top_level_eq("key=val"), Some(3));
    }

    #[test]
    fn top_level_eq_in_quotes() {
        assert!(find_top_level_eq("\"a=b\"").is_none());
    }

    #[test]
    fn top_level_eq_none() {
        assert!(find_top_level_eq("bareword").is_none());
    }

    // ── find_matching_open_brace ──

    #[test]
    fn matching_brace_simple() {
        assert_eq!(find_matching_open_brace("{content}"), Some(0));
    }

    #[test]
    fn matching_brace_with_prefix() {
        assert_eq!(find_matching_open_brace("text {content}"), Some(5));
    }

    #[test]
    fn matching_brace_nested() {
        assert_eq!(find_matching_open_brace("{outer {inner}}"), Some(0));
    }

    #[test]
    fn matching_brace_no_close() {
        assert!(find_matching_open_brace("no brace").is_none());
    }

    #[test]
    fn matching_brace_unbalanced() {
        // More } than {
        assert!(find_matching_open_brace("extra}").is_none());
    }

    #[test]
    fn matching_brace_empty() {
        assert!(find_matching_open_brace("").is_none());
    }

    // ── split_directive_args ──

    #[test]
    fn directive_args_simple() {
        let parts = split_directive_args("fig, counts=figure");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], ("fig".to_string(), "fig".to_string()));
        assert_eq!(parts[1].0, "counts");
        assert_eq!(parts[1].1, "figure");
    }

    #[test]
    fn directive_args_with_formats() {
        let parts = split_directive_args("sec, formats=[\"I\",\"1\"]");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].0, "formats");
    }

    #[test]
    fn directive_args_empty() {
        let parts = split_directive_args("");
        assert!(parts.is_empty());
    }

    // ── parse_format_list ──

    #[test]
    fn format_list_bracket_syntax() {
        let list = parse_format_list("[\"I\",\"1\",\"a\"]");
        assert_eq!(list, vec!["I", "1", "a"]);
    }

    #[test]
    fn format_list_single_token() {
        let list = parse_format_list("I");
        assert_eq!(list, vec!["I"]);
    }

    #[test]
    fn format_list_empty_brackets() {
        let list = parse_format_list("[]");
        assert!(list.is_empty());
    }

    // ── parse_init_directive ──

    #[test]
    fn hint_directive_simple() {
        let hints = parse_init_directive("figure, series=figs");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "series");
    }

    #[test]
    fn hint_directive_multiple_props() {
        let hints = parse_init_directive("figure, series=figs, label=\"Figure {n}\"");
        assert_eq!(hints.len(), 2);
    }

    #[test]
    fn hint_directive_no_props() {
        let hints = parse_init_directive("figure");
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_directive_only_selector_parts() {
        let hints = parse_init_directive("figure.wide");
        assert!(hints.is_empty());
    }

    #[test]
    fn hint_directive_with_attr_selector() {
        let hints = parse_init_directive("code-block[lang=\"rust\"], theme=dark");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].property, "theme");
    }

    // ── scan_delimited_token ──

    #[test]
    fn scan_delimited_stops_at_comma() {
        let chars: Vec<char> = "abc,def".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 3);
    }

    #[test]
    fn scan_delimited_skips_quoted_comma() {
        let chars: Vec<char> = "\"a,b\",c".chars().collect();
        assert_eq!(scan_delimited_token(&chars, 0), 5);
    }

    // ── split_kv ──

    #[test]
    fn split_kv_with_eq() {
        let (k, v) = split_kv("key=val");
        assert_eq!(k, "key");
        assert_eq!(v, "val");
    }

    #[test]
    fn split_kv_no_eq() {
        let (k, v) = split_kv("bare");
        assert_eq!(k, "bare");
        assert_eq!(v, "bare");
    }

    #[test]
    fn split_kv_quoted() {
        let (k, v) = split_kv("key=\"val\"");
        assert_eq!(k, "key");
        assert_eq!(v, "val");
    }

    // ── Unknown directive ──

    #[test]
    fn unknown_directive_ignored() {
        let mut attrs = Attrs::default();
        apply_directive(&mut attrs, "unknown_cmd", "arg1, arg2");
        assert!(attrs.declarations.is_empty());
        assert!(attrs.inits.is_empty());
    }



    // ── parse_attr_directive edge cases ──

    #[test]
    fn parse_attr_directive_no_parens() {
        let mut attrs = Attrs::default();
        let chars: Vec<char> = "@noparen rest".chars().collect();
        let consumed = parse_attr_directive(&chars, 0, &mut attrs);
        // Should skip the @noparen token
        assert!(consumed > 0);
    }



    // ── parse_init_directive edge cases ──

    #[test]
    fn parse_init_directive_no_properties() {
        // Just a selector with no property assignments
        let result = parse_init_directive("figure");
        assert!(result.is_empty());
    }

    // ── parse_prelude tests ──

    #[test]
    fn parse_prelude_basic() {
        let text = r#"
@var(fig, 0, format="1")
@init(figure, label="Figure {n}")
"#;
        let (decls, inits) = parse_prelude(text);
        assert!(!decls.is_empty(), "should produce declarations");
        assert!(!inits.is_empty(), "should produce inits");
        let has_var = decls.iter().any(|d| matches!(d, Declaration::VarDecl(_)));
        assert!(has_var, "should have a var declaration");
        let has_label = inits.iter().any(|i| i.property == "label");
        assert!(has_label, "should have label init");
    }

    #[test]
    fn parse_prelude_comments_and_blanks() {
        let text = "# this is a comment\n\n@init(heading, numbering=true)\n\n# another comment\n";
        let (decls, inits) = parse_prelude(text);
        assert!(decls.is_empty());
        assert_eq!(inits.len(), 1);
        assert_eq!(inits[0].property, "numbering");
    }

    #[test]
    fn parse_prelude_with_at_prefix() {
        let text = "@var(fig, 0, format=\"1\");\n@init(figure, label=\"Fig {n}\");\n";
        let (decls, inits) = parse_prelude(text);
        assert!(!decls.is_empty());
        assert!(!inits.is_empty());
    }

    #[test]
    fn parse_prelude_empty() {
        let (decls, inits) = parse_prelude("");
        assert!(decls.is_empty());
        assert!(inits.is_empty());
    }

}
