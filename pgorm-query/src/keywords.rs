//! PostgreSQL's keyword categories, and the one decision they feed: whether a
//! name part a caller supplied may be written bare at a given position.
//!
//! A lowercase name folds to itself, so writing it bare or quoted names the
//! same object — unless the word is a keyword. PostgreSQL sorts its keywords
//! into four categories (`src/include/parser/kwlist.h`), and the grammar
//! accepts each category bare in different productions:
//!
//! | Category | Bare as a column or schema (`ColId`) | Bare as a function or type (`type_function_name`) |
//! | --- | --- | --- |
//! | `UNRESERVED` | yes | yes |
//! | `COL_NAME` | yes | no — but some are the grammar's own type spellings or call forms |
//! | `TYPE_FUNC_NAME` | no | yes |
//! | `RESERVED` | no | no |
//!
//! A keyword the production does not accept is a syntax error at best, and at
//! worst a different statement: `not(1)` is a boolean NOT, `row(1)` a row
//! constructor, `distinct(1)` in first position a `SELECT DISTINCT`.
//! PostgreSQL's own `quote_ident()` therefore quotes every keyword that is not
//! `UNRESERVED`, and so does [`bare`], with two exceptions, each at one
//! position, where the bare keyword is what the caller means: the grammar's
//! type spellings in a type ([`TYPE_SPELLINGS`]) and its call forms in a
//! function call ([`CALL_FORMS`]).

/// A keyword category other than `UNRESERVED`, which the policy never needs
/// to distinguish from an ordinary name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    /// `COL_NAME_KEYWORD`: a column or schema name, not a function or type.
    ColName,
    /// `TYPE_FUNC_NAME_KEYWORD`: a function or type name, not a column.
    TypeFuncName,
    /// `RESERVED_KEYWORD`: bare only as a label after `AS` or a `.`.
    Reserved,
}

use Category::{ColName, Reserved, TypeFuncName};

/// Every keyword PostgreSQL 18.6 does not classify `UNRESERVED`, in byte
/// order, copied from its `src/include/parser/kwlist.h`.
///
/// 18.6 is the release of the libpg_query pgorm links (libpg_query 18.1.0,
/// through the pg_query fork the root manifest names), so the list the policy
/// reads is the one the parser-backed tests judge it by, and the test that
/// pins it to that scanner fails when the linked parser moves to another
/// release. The set is the same in PostgreSQL 17 and 19: the four words 18
/// added (`enforced`, `objects`, `period`, `virtual`) are unreserved, as was
/// the `recheck` it dropped. PostgreSQL 16 lacks the SQL/JSON words 17 added
/// and leaves `json` unreserved, and quoting a word an older server does not
/// reserve names the same object. A word a later release restricts is an
/// ordinary name here until the list is copied again.
const KEYWORDS: &[(&str, Category)] = &[
    ("all", Reserved),
    ("analyse", Reserved),
    ("analyze", Reserved),
    ("and", Reserved),
    ("any", Reserved),
    ("array", Reserved),
    ("as", Reserved),
    ("asc", Reserved),
    ("asymmetric", Reserved),
    ("authorization", TypeFuncName),
    ("between", ColName),
    ("bigint", ColName),
    ("binary", TypeFuncName),
    ("bit", ColName),
    ("boolean", ColName),
    ("both", Reserved),
    ("case", Reserved),
    ("cast", Reserved),
    ("char", ColName),
    ("character", ColName),
    ("check", Reserved),
    ("coalesce", ColName),
    ("collate", Reserved),
    ("collation", TypeFuncName),
    ("column", Reserved),
    ("concurrently", TypeFuncName),
    ("constraint", Reserved),
    ("create", Reserved),
    ("cross", TypeFuncName),
    ("current_catalog", Reserved),
    ("current_date", Reserved),
    ("current_role", Reserved),
    ("current_schema", TypeFuncName),
    ("current_time", Reserved),
    ("current_timestamp", Reserved),
    ("current_user", Reserved),
    ("dec", ColName),
    ("decimal", ColName),
    ("default", Reserved),
    ("deferrable", Reserved),
    ("desc", Reserved),
    ("distinct", Reserved),
    ("do", Reserved),
    ("else", Reserved),
    ("end", Reserved),
    ("except", Reserved),
    ("exists", ColName),
    ("extract", ColName),
    ("false", Reserved),
    ("fetch", Reserved),
    ("float", ColName),
    ("for", Reserved),
    ("foreign", Reserved),
    ("freeze", TypeFuncName),
    ("from", Reserved),
    ("full", TypeFuncName),
    ("grant", Reserved),
    ("greatest", ColName),
    ("group", Reserved),
    ("grouping", ColName),
    ("having", Reserved),
    ("ilike", TypeFuncName),
    ("in", Reserved),
    ("initially", Reserved),
    ("inner", TypeFuncName),
    ("inout", ColName),
    ("int", ColName),
    ("integer", ColName),
    ("intersect", Reserved),
    ("interval", ColName),
    ("into", Reserved),
    ("is", TypeFuncName),
    ("isnull", TypeFuncName),
    ("join", TypeFuncName),
    ("json", ColName),
    ("json_array", ColName),
    ("json_arrayagg", ColName),
    ("json_exists", ColName),
    ("json_object", ColName),
    ("json_objectagg", ColName),
    ("json_query", ColName),
    ("json_scalar", ColName),
    ("json_serialize", ColName),
    ("json_table", ColName),
    ("json_value", ColName),
    ("lateral", Reserved),
    ("leading", Reserved),
    ("least", ColName),
    ("left", TypeFuncName),
    ("like", TypeFuncName),
    ("limit", Reserved),
    ("localtime", Reserved),
    ("localtimestamp", Reserved),
    ("merge_action", ColName),
    ("national", ColName),
    ("natural", TypeFuncName),
    ("nchar", ColName),
    ("none", ColName),
    ("normalize", ColName),
    ("not", Reserved),
    ("notnull", TypeFuncName),
    ("null", Reserved),
    ("nullif", ColName),
    ("numeric", ColName),
    ("offset", Reserved),
    ("on", Reserved),
    ("only", Reserved),
    ("or", Reserved),
    ("order", Reserved),
    ("out", ColName),
    ("outer", TypeFuncName),
    ("overlaps", TypeFuncName),
    ("overlay", ColName),
    ("placing", Reserved),
    ("position", ColName),
    ("precision", ColName),
    ("primary", Reserved),
    ("real", ColName),
    ("references", Reserved),
    ("returning", Reserved),
    ("right", TypeFuncName),
    ("row", ColName),
    ("select", Reserved),
    ("session_user", Reserved),
    ("setof", ColName),
    ("similar", TypeFuncName),
    ("smallint", ColName),
    ("some", Reserved),
    ("substring", ColName),
    ("symmetric", Reserved),
    ("system_user", Reserved),
    ("table", Reserved),
    ("tablesample", TypeFuncName),
    ("then", Reserved),
    ("time", ColName),
    ("timestamp", ColName),
    ("to", Reserved),
    ("trailing", Reserved),
    ("treat", ColName),
    ("trim", ColName),
    ("true", Reserved),
    ("union", Reserved),
    ("unique", Reserved),
    ("user", Reserved),
    ("using", Reserved),
    ("values", ColName),
    ("varchar", ColName),
    ("variadic", Reserved),
    ("verbose", TypeFuncName),
    ("when", Reserved),
    ("where", Reserved),
    ("window", Reserved),
    ("with", Reserved),
    ("xmlattributes", ColName),
    ("xmlconcat", ColName),
    ("xmlelement", ColName),
    ("xmlexists", ColName),
    ("xmlforest", ColName),
    ("xmlnamespaces", ColName),
    ("xmlparse", ColName),
    ("xmlpi", ColName),
    ("xmlroot", ColName),
    ("xmlserialize", ColName),
    ("xmltable", ColName),
];

/// The category of `word`, or `None` for an ordinary name or an `UNRESERVED`
/// keyword. Keywords are lowercase; a word with any other character is none.
pub(crate) fn category(word: &str) -> Option<Category> {
    KEYWORDS
        .binary_search_by(|(keyword, _)| keyword.cmp(&word))
        .ok()
        .map(|index| KEYWORDS[index].1)
}

/// Where a name part is written. The positions differ in which bare keyword,
/// if any, is the grammar's own form rather than a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Position {
    /// A type named without a schema, array or not: a cast target, a column
    /// type, an enum type.
    Type,
    /// Any other part of a type name: its schema, or its name after one.
    QualifiedType,
    /// A function's name in a call, in an expression or a `FROM` list.
    Function,
    /// An index access method (`USING method`).
    AccessMethod,
}

/// The `COL_NAME` keywords PostgreSQL's grammar reads, bare in a type
/// position, as one of its own type spellings, each resolving to the
/// catalogue type after the arrow: `bigint` → `int8`, `bit` → `bit`,
/// `boolean` → `bool`, `char` and `character` and `nchar` → `bpchar`, `dec`
/// and `decimal` and `numeric` → `numeric`, `float` → `float8`, `int` and
/// `integer` → `int4`, `interval`, `json`, `real` → `float4`, `smallint` →
/// `int2`, `time`, `timestamp`, `varchar`. Quoted, most of them name no type
/// at all, and `"char"` names a different one.
///
/// `national` and `precision` are not here: each is only half of a type
/// spelling (`national character`, `double precision`) and a syntax error on
/// its own, so a part of either name is quoted like any other keyword.
pub(crate) const TYPE_SPELLINGS: &[&str] = &[
    "bigint",
    "bit",
    "boolean",
    "char",
    "character",
    "dec",
    "decimal",
    "float",
    "int",
    "integer",
    "interval",
    "json",
    "nchar",
    "numeric",
    "real",
    "smallint",
    "time",
    "timestamp",
    "varchar",
];

/// The `COL_NAME` keywords a caller names as a function and means as the
/// grammar's own call form: `coalesce(..)`, `greatest(..)`, `least(..)` and
/// `nullif(..)` are expressions the parser builds itself, not functions in
/// `pg_proc`, so a quoted `"coalesce"(..)` names a function that does not
/// exist.
pub(crate) const CALL_FORMS: &[&str] = &["coalesce", "greatest", "least", "nullif"];

/// The `UNRESERVED` keyword a bare call cannot spell: in an expression,
/// `operator(` opens the grammar's qualified-operator syntax,
/// `OPERATOR(schema.op)`, so `operator(1, 2)` is a syntax error where
/// `"operator"(1, 2)` calls a function of that name. Every other unreserved
/// keyword reads bare as the same name at every position, which the
/// identifier oracle's keyword sweep holds.
pub(crate) const NOT_BARE_CALLS: &[&str] = &["operator"];

/// Whether `part` may be written bare at `position`: it is a lowercase
/// identifier, and either no keyword the position's grammar restricts —
/// every `COL_NAME`, `TYPE_FUNC_NAME` and `RESERVED` keyword, and
/// [`NOT_BARE_CALLS`] in a call — or one the position reads as the
/// grammar's own form ([`TYPE_SPELLINGS`] in a type, [`CALL_FORMS`] in a
/// call).
///
/// Anything else is quoted by the caller of this function, and a quoted
/// lowercase word names exactly what the bare word would have named had it
/// not been a keyword.
pub(crate) fn bare(part: &str, position: Position) -> bool {
    let mut chars = part.chars();
    let identifier = matches!(chars.next(), Some('a'..='z' | '_'))
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'));
    identifier
        && match category(part) {
            None => position != Position::Function || !NOT_BARE_CALLS.contains(&part),
            Some(ColName) => match position {
                Position::Type => TYPE_SPELLINGS.contains(&part),
                Position::Function => CALL_FORMS.contains(&part),
                Position::QualifiedType | Position::AccessMethod => false,
            },
            Some(TypeFuncName | Reserved) => false,
        }
}

#[cfg(test)]
mod tests {
    use pg_query::protobuf::{KeywordKind, Token};

    use super::*;

    /// The category the linked scanner gives `word`.
    fn scanned(word: &str) -> KeywordKind {
        let tokens = pg_query::scan(word).expect("a word scans").tokens;
        assert_eq!(tokens.len(), 1, "`{word}` is one token");
        KeywordKind::try_from(tokens[0].keyword_kind).expect("a known keyword kind")
    }

    fn kind(category: Category) -> KeywordKind {
        match category {
            ColName => KeywordKind::ColNameKeyword,
            TypeFuncName => KeywordKind::TypeFuncNameKeyword,
            Reserved => KeywordKind::ReservedKeyword,
        }
    }

    // [spec:pgorm:def:sql.types.type-name+9/test]    the list is the linked
    // parser's, both ways: every entry scans as its category, and every
    // keyword the scanner restricts is an entry
    #[test]
    fn keywords_are_the_linked_scanners() {
        const COPIED_FROM: i32 = 180_006;
        let version = pg_query::parse("SELECT 1", pg_query::ParserOptions::DEFAULT)
            .expect("parses")
            .protobuf
            .version;
        assert_eq!(
            version, COPIED_FROM,
            "the linked libpg_query moved; re-copy KEYWORDS from its kwlist.h"
        );
        assert!(
            KEYWORDS.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "byte order"
        );
        for (word, category) in KEYWORDS {
            assert_eq!(scanned(word), kind(*category), "`{word}`");
        }
        let mut restricted = 0;
        for value in 0..2048 {
            let Ok(token) = Token::try_from(value) else {
                continue;
            };
            let name = token.as_str_name().to_ascii_lowercase();
            let word = name.strip_suffix("_p").unwrap_or(&name);
            let Ok(scan) = pg_query::scan(word) else {
                continue;
            };
            let [only] = scan.tokens.as_slice() else {
                continue;
            };
            let found = KeywordKind::try_from(only.keyword_kind).expect("a known keyword kind");
            if matches!(
                found,
                KeywordKind::NoKeyword | KeywordKind::UnreservedKeyword
            ) {
                continue;
            }
            restricted += 1;
            assert_eq!(category(word).map(kind), Some(found), "`{word}` is missing");
        }
        assert_eq!(restricted, KEYWORDS.len());
    }
}
