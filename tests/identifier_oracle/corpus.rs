//! The hostile-name corpus every registered site is held to.
//!
//! Each entry is a name a caller could hand an identifier API, chosen for what
//! it would do to the statement if the site wrote it as text rather than as a
//! name: close the quote, end the statement, open a comment, masquerade as a
//! placeholder or a dollar-quoted body, open a string, or merely be the kind of
//! name a careless quoting policy treats as SQL (a keyword, the empty string,
//! a name at the identifier-length boundary).
//!
//! NUL is not here. No PostgreSQL identifier can hold one and the parser
//! cannot parse a string that contains one, so its leg is judged separately,
//! by [`NUL_NAME`], against each site's declared NUL behaviour.

/// A name carrying NUL, for the per-site NUL leg.
pub const NUL_NAME: &str = "nul\0name";

/// One corpus entry: the name, and a label for failure messages that stays
/// readable when the name itself is whitespace or invisible.
#[derive(Clone, Debug)]
pub struct Hostile {
    pub label: &'static str,
    pub name: String,
}

fn entry(label: &'static str, name: impl Into<String>) -> Hostile {
    Hostile {
        label,
        name: name.into(),
    }
}

/// The corpus. Order is the order failures are reported in.
pub fn corpus() -> Vec<Hostile> {
    vec![
        // The identifier delimiter, alone, doubled, embedded, and after a
        // backslash — the escape registry prqlc 0.13.14 got wrong.
        entry("quote", "\""),
        entry("doubled-quote", "\"\""),
        entry("tripled-quote", "\"\"\""),
        entry("embedded-quote", "a\"b"),
        entry("backslash-quote", "\\\""),
        // The exfiltration payload of f48c6425: under the stock escaper it
        // closed the identifier and projected a subquery beside the row.
        entry("f48c6425-exfiltration", "x\\\" , (SELECT 1) AS \"leak"),
        entry("close-and-stack", "x\"; DROP TABLE t; --"),
        entry("close-and-or", "x\" OR \"1\"=\"1"),
        // Statement and comment punctuation.
        entry("semicolon", ";"),
        entry("line-comment", "--"),
        entry("block-comment-open", "/*"),
        entry("block-comment-close", "*/"),
        // Placeholders and dollar quoting.
        entry("placeholder", "$1"),
        entry("dollar-dollar", "$$"),
        entry("dollar-tag", "$tag$"),
        // String delimiters and escapes.
        entry("single-quote", "'"),
        entry("escape-string-open", "E'"),
        entry("backslash", "\\"),
        entry("unicode-escape-prefix", "U&\"x"),
        // Whitespace.
        entry("space", " "),
        entry("inner-space", "a b"),
        entry("newline", "\n"),
        entry("inner-newline", "line\nbreak"),
        entry("tab", "\t"),
        entry("carriage-return", "a\rb"),
        // Unicode: a combining mark, a right-to-left override, full-width
        // and typographic quotes, and a zero-width joiner.
        entry("combining-mark", "e\u{0301}"),
        entry("right-to-left-override", "\u{202E}kcatta"),
        entry("full-width-quote", "\u{FF02}x\u{FF02}"),
        entry("typographic-quotes", "\u{201C}x\u{201D}"),
        entry("zero-width-joiner", "a\u{200D}b"),
        // Case the server would fold if the name travelled bare.
        entry("mixed-case", "MixedCase"),
        // Names that mean something to a layer between the caller and the
        // grammar: the wildcard, a dotted path, and the pipeline's own
        // binding-name spelling (`table_N`).
        entry("asterisk", "*"),
        entry("dotted", "a.b"),
        entry("binding-name", "table_0"),
        // Keywords: reserved (`select`, `from`, `user`, `not`), a column-name
        // keyword that is also type sugar (`integer`) and a type/function-name
        // keyword that is also an ordinary function (`left`).
        entry("keyword-select", "select"),
        entry("keyword-from", "from"),
        entry("keyword-user", "user"),
        entry("keyword-not", "not"),
        entry("keyword-integer", "integer"),
        entry("keyword-left", "left"),
        // Keywords whose bare call is the grammar's own expression: a
        // COALESCE (a call form the type-name policy keeps bare), a row
        // constructor, a `SELECT DISTINCT` when first in the projection, and
        // an inheritance marker before a FROM function (all three quoted).
        entry("keyword-coalesce", "coalesce"),
        entry("keyword-row", "row"),
        entry("keyword-distinct", "distinct"),
        entry("keyword-only", "only"),
        // The empty name.
        entry("empty", ""),
        // The identifier-length boundary (NAMEDATALEN - 1 = 63 bytes), and a
        // 64-byte name whose 63rd byte falls inside a two-byte character.
        entry("63-bytes", "n".repeat(63)),
        entry("64-bytes", "n".repeat(64)),
        entry("64-bytes-split-char", format!("{}\u{00E9}", "n".repeat(62))),
    ]
}

/// Every keyword the linked scanner knows, in every category,
/// for the sweep that holds `TypeName`'s part policy to the whole keyword
/// list rather than to the corpus's handful.
///
/// Read off the scanner rather than copied from `kwlist.h`, so the sweep and
/// the list pgorm-query embeds are independent: each keyword token's name,
/// lowercased and stripped of the `_P` suffix the grammar uses to dodge C
/// names, is scanned back, and kept when the scanner reads it as a keyword.
pub fn scanner_keywords() -> Vec<Hostile> {
    use pgsql_syntax::protobuf::{KeywordKind, Token};

    let mut words = std::collections::BTreeSet::new();
    for value in 0..4096 {
        let Ok(token) = Token::try_from(value) else {
            continue;
        };
        let name = token.as_str_name().to_ascii_lowercase();
        let word = name.strip_suffix("_p").unwrap_or(&name);
        let Ok(scanned) = pgsql_syntax::scan(word) else {
            continue;
        };
        if let [only] = scanned.tokens.as_slice()
            && only.keyword_kind != KeywordKind::NoKeyword as i32
        {
            words.insert(word.to_owned());
        }
    }
    words
        .into_iter()
        .map(|word| entry("scanner-keyword", word))
        .collect()
}
