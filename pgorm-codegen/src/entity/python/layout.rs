//! Python laid out in black's layout: a bracket's contents on its line where
//! they fit in 88 columns, on a line of their own where those fit, and
//! otherwise one to a line, each followed by a comma, a lone item taking
//! none; strings double-quoted unless that needs more escapes; and an
//! import's names in isort's order, a name to a line where they do not fit.

/// The width black lays lines out in.
const WIDTH: usize = 88;

/// A piece of Python whose layout depends on the room it has.
#[derive(Clone, Debug)]
pub(crate) enum Py {
    /// Text laid out as it is: a name, a literal, an expression.
    Atom(String),
    /// `head`, which opens a bracket, its items, and `close`.
    Bracket {
        head: String,
        items: Vec<Py>,
        close: &'static str,
    },
    /// `prefix` — a dict's key and colon, an argument's name and `=`, an
    /// assignment's target — before a piece laid out after it.
    Prefixed(String, Box<Py>),
}

impl Py {
    pub(crate) fn call(callee: &str, items: Vec<Py>) -> Self {
        Self::Bracket {
            head: format!("{callee}("),
            items,
            close: ")",
        }
    }

    pub(crate) fn list(items: Vec<Py>) -> Self {
        Self::Bracket {
            head: "[".to_owned(),
            items,
            close: "]",
        }
    }

    /// A tuple, a lone item followed by its comma.
    pub(crate) fn tuple(items: Vec<Py>) -> Self {
        match <[Py; 1]>::try_from(items) {
            Ok([only]) => Self::Atom(format!("({},)", only.flat())),
            Err(items) => Self::Bracket {
                head: "(".to_owned(),
                items,
                close: ")",
            },
        }
    }

    pub(crate) fn dict(entries: Vec<(String, Py)>) -> Self {
        Self::Bracket {
            head: "{".to_owned(),
            items: entries
                .into_iter()
                .map(|(key, value)| Self::Prefixed(format!("{}: ", string(&key)), Box::new(value)))
                .collect(),
            close: "}",
        }
    }

    pub(crate) fn keyword(name: &str, value: Py) -> Self {
        Self::Prefixed(format!("{name}="), Box::new(value))
    }

    /// The piece on one line, however long.
    pub(crate) fn flat(&self) -> String {
        match self {
            Self::Atom(text) => text.clone(),
            Self::Bracket { head, items, close } => {
                let inner: Vec<String> = items.iter().map(Self::flat).collect();
                format!("{head}{}{close}", inner.join(", "))
            }
            Self::Prefixed(prefix, value) => format!("{prefix}{}", value.flat()),
        }
    }

    /// The piece starting `before` columns into a line indented `indent`,
    /// with `after` columns following it on its last line; its first line
    /// carries no indentation of its own.
    pub(crate) fn render(&self, indent: usize, before: usize, after: usize) -> String {
        match self {
            Self::Atom(text) => text.clone(),
            Self::Prefixed(prefix, value) => {
                format!(
                    "{prefix}{}",
                    value.render(indent, before + prefix.len(), after)
                )
            }
            Self::Bracket { head, items, close } => {
                let line = self.flat();
                if indent + before + line.chars().count() + after <= WIDTH || items.is_empty() {
                    return line;
                }
                let inner = indent + 4;
                let pad = " ".repeat(inner);
                let end = format!("{}{close}", " ".repeat(indent));
                if let [only] = items.as_slice() {
                    return format!("{head}\n{pad}{}\n{end}", only.render(inner, 0, 0));
                }
                let joined: Vec<String> = items.iter().map(Self::flat).collect();
                let joined = joined.join(", ");
                if inner + joined.chars().count() <= WIDTH {
                    return format!("{head}\n{pad}{joined}\n{end}");
                }
                let body: Vec<String> = items
                    .iter()
                    .map(|item| format!("{pad}{},", item.render(inner, 0, 1)))
                    .collect();
                format!("{head}\n{}\n{end}", body.join("\n"))
            }
        }
    }
}

/// `value` as a Python string literal: double-quoted unless single quotes
/// need fewer escapes, as black writes it, with Python's escapes for a
/// backslash, the quote, and the control characters.
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
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                literal.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => literal.push(c),
        }
    }
    literal.push(quote);
    literal
}

/// `from module import names`, the names in isort's order, on one line
/// where it fits and otherwise a name to a line in parentheses.
pub(crate) fn from_import(module: &str, names: &[&str]) -> String {
    let mut ordered: Vec<&str> = names.to_vec();
    ordered.sort_by_key(|name| isort_key(name));
    ordered.dedup();
    let line = format!("from {module} import {}", ordered.join(", "));
    if line.chars().count() <= WIDTH {
        return line;
    }
    let body: Vec<String> = ordered.iter().map(|name| format!("    {name},")).collect();
    format!("from {module} import (\n{}\n)", body.join("\n"))
}

/// isort's order of the names one import gives, `order_by_type`:
/// constants, then classes, then functions and the rest, each by name.
fn isort_key(name: &str) -> (u8, String) {
    let bare = name.split(" as ").next().unwrap_or(name);
    let upper = bare.chars().count() > 1 && bare == bare.to_uppercase();
    let class = bare.chars().next().is_some_and(char::is_uppercase);
    let rank = match (upper, class) {
        (true, _) => 0,
        (false, true) => 1,
        (false, false) => 2,
    };
    (rank, bare.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:pgorm:req:codegen.models.layout/test]    a declared-model module's brackets are laid out as black lays them out: on one line, the contents on a line of their own, one to a line with a comma after each, a lone item with none
    #[test]
    fn brackets_take_blacks_layout() {
        let column = |options: &str| Py::Atom(format!("Column(\"i32\", {options})"));
        let short = Py::Prefixed(
            "Bakery = ".to_owned(),
            Box::new(Py::call(
                "Model",
                vec![
                    Py::Atom(string("bakery")),
                    Py::dict(vec![("id".to_owned(), column("primary_key=True"))]),
                ],
            )),
        );
        assert_eq!(
            short.render(0, 0, 0),
            "Bakery = Model(\"bakery\", {\"id\": Column(\"i32\", primary_key=True)})"
        );
        let relation = Py::Prefixed(
            "cake_baker_cake = ".to_owned(),
            Box::new(Py::call(
                "CakeBaker.belongs_to",
                vec![
                    Py::Atom("Cake".to_owned()),
                    Py::keyword("from_", Py::Atom(string("cake_id"))),
                    Py::keyword("to", Py::Atom(string("id"))),
                    Py::keyword("on_delete", Py::Atom(string("cascade"))),
                ],
            )),
        );
        assert_eq!(
            relation.render(0, 0, 0),
            "cake_baker_cake = CakeBaker.belongs_to(\n    Cake, from_=\"cake_id\", to=\"id\", \
             on_delete=\"cascade\"\n)"
        );
        let lone = Py::Prefixed(
            "y = ".to_owned(),
            Box::new(Py::dict(vec![(
                "a".repeat(29),
                Py::call(
                    "Column",
                    vec![
                        Py::Atom(string("i32")),
                        Py::keyword("primary_key", Py::Atom("True".to_owned())),
                        Py::keyword("auto_increment", Py::Atom("True".to_owned())),
                        Py::keyword("nullablex", Py::Atom("True".to_owned())),
                    ],
                ),
            )])),
        );
        assert_eq!(
            lone.render(0, 0, 0),
            format!(
                "y = {{\n    \"{}\": Column(\n        \"i32\", primary_key=True, \
                 auto_increment=True, nullablex=True\n    )\n}}",
                "a".repeat(29)
            )
        );
    }

    // [spec:pgorm:req:codegen.models.layout/test]    strings are double-quoted unless that needs more escapes, and an import's names take isort's order
    #[test]
    fn strings_and_imports_take_blacks_and_isorts_layout() {
        assert_eq!(string("cake"), "\"cake\"");
        assert_eq!(string("say \"hi\""), "'say \"hi\"'");
        assert_eq!(string("it's \"x\""), "'it\\'s \"x\"'");
        assert_eq!(string("a\\b\n"), "\"a\\\\b\\n\"");
        assert_eq!(
            from_import("pgorm", &["col", "Model", "FLAVOR", "Column", "func"]),
            "from pgorm import FLAVOR, Column, Model, col, func"
        );
        let many: Vec<String> = (0..12).map(|n| format!("Name{n:02}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(from_import("pgorm", &names).starts_with("from pgorm import (\n    Name00,\n"));
    }
}
