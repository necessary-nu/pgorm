//! TypeScript laid out in deno fmt's and prettier's layout: 80 columns, two
//! spaces a level, a bracket's items on its line where they fit and
//! otherwise one to a line with a comma after each, a call whose last
//! argument is its one object or array hugging it, strings double-quoted
//! unless that needs more escapes, and named imports sorted as deno fmt
//! sorts them.

/// The width deno fmt and prettier lay lines out in.
const WIDTH: usize = 80;

/// A piece of TypeScript whose layout depends on the room it has.
#[derive(Clone, Debug)]
pub(crate) enum Ts {
    /// Text laid out as it is: a name, a literal, an expression.
    Atom(String),
    /// `callee(args)`.
    Call(String, Vec<Ts>),
    /// `{ key: value }`, each key written as a property is.
    Object(Vec<(String, Ts)>),
    Array(Vec<Ts>),
}

impl Ts {
    /// An object of `entries`, each key a property name.
    pub(crate) fn object(entries: Vec<(String, Ts)>) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (property(&key), value))
                .collect(),
        )
    }

    fn huggable(&self) -> bool {
        match self {
            Self::Object(entries) => !entries.is_empty(),
            Self::Array(items) => !items.is_empty(),
            _ => false,
        }
    }

    /// The piece on one line, however long.
    pub(crate) fn flat(&self) -> String {
        match self {
            Self::Atom(text) => text.clone(),
            Self::Call(callee, args) => {
                let args: Vec<String> = args.iter().map(Self::flat).collect();
                format!("{callee}({})", args.join(", "))
            }
            Self::Object(entries) if entries.is_empty() => "{}".to_owned(),
            Self::Object(entries) => {
                let entries: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| format!("{key}: {}", value.flat()))
                    .collect();
                format!("{{ {} }}", entries.join(", "))
            }
            Self::Array(items) => {
                let items: Vec<String> = items.iter().map(Self::flat).collect();
                format!("[{}]", items.join(", "))
            }
        }
    }

    /// The piece starting `before` columns into a line indented `indent`,
    /// with `after` columns following it where it fits on one.
    pub(crate) fn render(&self, indent: usize, before: usize, after: usize) -> String {
        let line = self.flat();
        if indent + before + line.chars().count() + after <= WIDTH {
            return line;
        }
        self.broken(indent, before)
    }

    /// The piece broken over lines, whether or not it fits on one: an
    /// object or array an entry to a line, a call hugging its one object or
    /// array where its other arguments leave room on its first line, and
    /// otherwise an argument to a line.
    fn broken(&self, indent: usize, before: usize) -> String {
        let inner = indent + 2;
        let pad = " ".repeat(inner);
        let end = " ".repeat(indent);
        match self {
            Self::Atom(text) => text.clone(),
            Self::Object(entries) if !entries.is_empty() => {
                let body: Vec<String> = entries
                    .iter()
                    .map(|(key, value)| {
                        let prefix = format!("{key}: ");
                        format!("{pad}{prefix}{},", value.render(inner, prefix.len(), 1))
                    })
                    .collect();
                format!("{{\n{}\n{end}}}", body.join("\n"))
            }
            Self::Array(items) if !items.is_empty() => {
                let body: Vec<String> = items
                    .iter()
                    .map(|item| format!("{pad}{},", item.render(inner, 0, 1)))
                    .collect();
                format!("[\n{}\n{end}]", body.join("\n"))
            }
            Self::Object(_) | Self::Array(_) => self.flat(),
            Self::Call(callee, args) => {
                if let Some((last, rest)) = args.split_last()
                    && last.huggable()
                    && !rest.iter().any(Self::huggable)
                {
                    let head: Vec<String> = rest.iter().map(Self::flat).collect();
                    let mut opening = format!("{callee}(");
                    for item in &head {
                        opening.push_str(item);
                        opening.push_str(", ");
                    }
                    if indent + before + opening.chars().count() < WIDTH {
                        let hugged = last.broken(indent, before + opening.chars().count());
                        return format!("{opening}{hugged})");
                    }
                }
                if args.is_empty() {
                    return self.flat();
                }
                let body: Vec<String> = args
                    .iter()
                    .map(|arg| format!("{pad}{},", arg.render(inner, 0, 1)))
                    .collect();
                format!("{callee}(\n{}\n{end})", body.join("\n"))
            }
        }
    }
}

/// `key` as an object's property: bare where it is an identifier, quoted
/// otherwise.
pub(crate) fn property(key: &str) -> String {
    let mut chars = key.chars();
    let bare = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if bare { key.to_owned() } else { string(key) }
}

/// `value` as a string literal: double-quoted unless single quotes need
/// fewer escapes, with JavaScript's escapes for a backslash, the quote and
/// the control characters.
pub(crate) fn string(value: &str) -> String {
    let count = |quote: char| value.chars().filter(|c| *c == quote).count();
    let quote = if count('"') > count('\'') { '\'' } else { '"' };
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push(quote);
    for c in value.chars() {
        match c {
            '\\' => literal.push_str("\\\\"),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            c if c == quote => {
                literal.push('\\');
                literal.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f || c == '\u{2028}' || c == '\u{2029}' => {
                literal.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => literal.push(c),
        }
    }
    literal.push(quote);
    literal
}

/// `import { names } from "source";`, the names sorted as deno fmt sorts
/// them, case-insensitively, on one line where it fits and otherwise a name
/// to a line.
pub(crate) fn import(names: &[&str], source: &str) -> String {
    let mut ordered: Vec<&str> = names.to_vec();
    ordered.sort_by_key(|name| (name.to_lowercase(), name.to_string()));
    ordered.dedup();
    let from = format!("}} from {};", string(source));
    let line = format!("import {{ {} {from}", ordered.join(", "));
    if line.chars().count() <= WIDTH {
        return line;
    }
    let body: Vec<String> = ordered.iter().map(|name| format!("  {name},")).collect();
    format!("import {{\n{}\n{from}", body.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:codegen.models.layout/test]    a declared-model module is laid out as deno fmt and prettier lay it out: within 80 columns, a call hugging its one object or array argument, one item to a line with a comma after each otherwise
    #[test]
    fn calls_take_deno_fmts_layout() {
        let relation = Ts::Call(
            "CakeBaker.belongsTo".to_owned(),
            vec![
                Ts::Atom("Cake".to_owned()),
                Ts::object(vec![
                    ("from".to_owned(), Ts::Atom(string("cakeId"))),
                    ("to".to_owned(), Ts::Atom(string("id"))),
                    ("onDelete".to_owned(), Ts::Atom(string("cascade"))),
                ]),
            ],
        );
        assert_eq!(
            relation.render(0, 29, 1),
            "CakeBaker.belongsTo(Cake, {\n  from: \"cakeId\",\n  to: \"id\",\n  onDelete: \
             \"cascade\",\n})"
        );
        let short = Ts::Call("column".to_owned(), vec![Ts::Atom(string("i32"))]);
        assert_eq!(short.render(4, 4, 1), "column(\"i32\")");
        let disjoint = Ts::Call(
            "disjoint".to_owned(),
            vec![
                Ts::Atom(string("payment")),
                Ts::Atom("Payment".to_owned()),
                Ts::object(vec![
                    ("card".to_owned(), Ts::Atom("CardPayment".to_owned())),
                    ("cash".to_owned(), Ts::Atom("null".to_owned())),
                ]),
                Ts::object(vec![("schemaName".to_owned(), Ts::Atom(string("public")))]),
            ],
        );
        assert_eq!(
            disjoint.render(0, 31, 1),
            "disjoint(\n  \"payment\",\n  Payment,\n  { card: CardPayment, cash: null },\n  { \
             schemaName: \"public\" },\n)"
        );
        assert_eq!(property("glutenFree"), "glutenFree");
        assert_eq!(property("display name"), "\"display name\"");
    }

    // [spec:pgorm:req:codegen.models.layout/test]    named imports are sorted case-insensitively, a name to a line where they do not fit
    #[test]
    fn imports_are_sorted_as_deno_fmt_sorts_them() {
        assert_eq!(
            import(
                &["TypeName", "model", "column", "DataType"],
                "@necessary/pgorm"
            ),
            "import { column, DataType, model, TypeName } from \"@necessary/pgorm\";"
        );
        let long = import(
            &[
                "column", "DataType", "Decimal", "disjoint", "model", "oneOf", "TypeName", "func",
            ],
            "@necessary/pgorm",
        );
        assert!(
            long.starts_with("import {\n  column,\n  DataType,\n"),
            "{long}"
        );
        assert_eq!(string("say \"hi\""), "'say \"hi\"'");
    }
}
