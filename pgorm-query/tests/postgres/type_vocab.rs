use super::*;
use crate::oracle::{assert_eq, assert_eq_unparsed};
use pgorm_query::error::{Error, TemplateError};
use std::sync::Arc;

// [spec:pgorm:def:sql.types.column-ref+1/test]    the seven forms and what `IntoColumnRef` maps onto
// them
#[test]
fn into_column_ref_maps_every_form() {
    assert_eq!(
        Glyph::Id.into_column_ref(),
        ColumnRef::Column(Glyph::Id.into_name())
    );
    assert_eq!(
        (Glyph::Table, Glyph::Id).into_column_ref(),
        ColumnRef::TableColumn(Glyph::Table.into_name(), Glyph::Id.into_name())
    );
    assert_eq!(
        (Name::runtime("schema"), Glyph::Table, Glyph::Id).into_column_ref(),
        ColumnRef::SchemaTableColumn(
            Name::runtime("schema"),
            Glyph::Table.into_name(),
            Glyph::Id.into_name()
        )
    );
    assert_eq!(Asterisk.into_column_ref(), ColumnRef::Asterisk);
    assert_eq!(
        (Glyph::Table, Asterisk).into_column_ref(),
        ColumnRef::TableAsterisk(Glyph::Table.into_name())
    );
    assert_eq!(
        (ReturningRow::Old, Glyph::Id).into_column_ref(),
        ColumnRef::RowColumn(ReturningRow::Old, Glyph::Id.into_name())
    );
    assert_eq!(
        (ReturningRow::New, Asterisk).into_column_ref(),
        ColumnRef::RowAsterisk(ReturningRow::New)
    );

    // An existing ColumnRef passes through unchanged.
    assert_eq!(ColumnRef::Asterisk.into_column_ref(), ColumnRef::Asterisk);
}

// [spec:pgorm:def:sql.types.column-ref+1/test]    each table form is renderable; the two version
// forms render in `returning.rs`, the one list that resolves them
#[test]
fn every_column_ref_form_renders() {
    assert_eq!(
        Query::select()
            .column(ColumnRef::Column(Glyph::Id.into_name()))
            .column(ColumnRef::TableColumn(
                Glyph::Table.into_name(),
                Glyph::Id.into_name()
            ))
            .column(ColumnRef::SchemaTableColumn(
                Name::runtime("schema"),
                Glyph::Table.into_name(),
                Glyph::Id.into_name()
            ))
            .column(ColumnRef::Asterisk)
            .column(ColumnRef::TableAsterisk(Glyph::Table.into_name()))
            .to_string(),
        r#"SELECT "id", "glyph"."id", "schema"."glyph"."id", *, "glyph".*"#
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    `IntoTableName` maps iden / 2-tuple
#[test]
fn into_table_name_maps_the_two_forms() {
    assert_eq!(
        Glyph::Table.into_table_name(),
        TableName::Table(Glyph::Table.into_name())
    );
    assert_eq!(
        (Name::runtime("schema"), Glyph::Table).into_table_name(),
        TableName::SchemaTable(Name::runtime("schema"), Glyph::Table.into_name())
    );
}

/// A table name that knows its schema, as an entity's does.
struct Ledger;

impl SqlName for Ledger {
    fn unquoted(&self, s: &mut dyn std::fmt::Write) {
        write!(s, "ledger").unwrap();
    }

    fn table_schema(&self) -> Option<&str> {
        Some("books")
    }
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    a bare name that answers a schema is qualified
// by it in every table position, a column of it staying qualified by the table alone
#[test]
fn a_name_with_a_schema_qualifies_every_table() {
    assert_eq!(
        Ledger.into_table_name(),
        TableName::SchemaTable(Name::runtime("books"), Ledger.into_name())
    );
    assert_eq!(
        Ledger.into_name().into_table_name(),
        Ledger.into_table_name()
    );
    assert_eq!(
        (Name::runtime("other"), Ledger).into_table_name(),
        TableName::SchemaTable(Name::runtime("other"), Ledger.into_name())
    );
    assert_eq!(
        Query::select()
            .column((Ledger, Glyph::Id))
            .from(Ledger)
            .inner_join(
                Glyph::Table,
                Expr::col((Glyph::Table, Glyph::Id)).equals((Ledger, Glyph::Id))
            )
            .to_string(),
        r#"SELECT "ledger"."id" FROM "books"."ledger" INNER JOIN "glyph" ON "glyph"."id" = "ledger"."id""#
    );
    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(Glyph::Table)
            .left_join(Ledger, Expr::val(true).eq(true))
            .to_string(),
        r#"SELECT * FROM "glyph" LEFT JOIN "books"."ledger" ON TRUE = TRUE"#
    );
    assert_eq!(
        Query::insert()
            .into_table(Ledger)
            .columns([Glyph::Id])
            .values_panic([1.into()])
            .to_string(),
        r#"INSERT INTO "books"."ledger" ("id") VALUES (1)"#
    );
    assert_eq!(
        Query::update()
            .table(Ledger)
            .value(Glyph::Id, 2)
            .to_string(),
        r#"UPDATE "books"."ledger" SET "id" = 2"#
    );
    assert_eq!(
        Query::delete().from_table(Ledger).to_string(),
        r#"DELETE FROM "books"."ledger""#
    );
    assert_eq!(
        Table::truncate(Ledger).to_string(),
        r#"TRUNCATE TABLE "books"."ledger""#
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    `IntoNamedTable` maps the same spellings to an
// unaliased named table, and a `TableName` or `NamedTable` passes through
#[test]
fn into_named_table_maps_the_named_forms() {
    let unaliased = |name| NamedTable { name, alias: None };

    assert_eq!(
        Glyph::Table.into_named_table(),
        unaliased(TableName::Table(Glyph::Table.into_name()))
    );
    assert_eq!(
        (Name::runtime("schema"), Glyph::Table).into_named_table(),
        unaliased(TableName::SchemaTable(
            Name::runtime("schema"),
            Glyph::Table.into_name()
        ))
    );
    assert_eq!(
        Glyph::Table.into_table_name().into_named_table(),
        Glyph::Table.into_named_table()
    );
    assert_eq!(
        Glyph::Table
            .into_named_table()
            .alias(Name::runtime("g"))
            .into_named_table(),
        Glyph::Table.into_named_table().alias(Name::runtime("g"))
    );
    assert_eq!(
        NamedTable::from(Glyph::Table.into_table_name()),
        Glyph::Table.into_named_table()
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    `IntoFromItem` widens every named spelling, and
// a `TableName` or `NamedTable` converts infallibly
#[test]
fn into_from_item_maps_the_named_forms() {
    assert_eq!(
        Glyph::Table.into_from_item(),
        FromItem::Table(NamedTable {
            name: TableName::Table(Glyph::Table.into_name()),
            alias: None,
        })
    );
    assert_eq!(
        (Name::runtime("schema"), Glyph::Table).into_from_item(),
        FromItem::Table(NamedTable {
            name: TableName::SchemaTable(Name::runtime("schema"), Glyph::Table.into_name()),
            alias: None,
        })
    );
    assert_eq!(
        FromItem::from((Name::runtime("schema"), Glyph::Table).into_table_name()),
        (Name::runtime("schema"), Glyph::Table).into_from_item()
    );
    assert_eq!(
        FromItem::from(Glyph::Table.into_named_table().alias(Name::runtime("g"))),
        Glyph::Table.into_from_item().alias(Name::runtime("g"))
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    `alias` binds an alias and replaces an existing
// one, on the named form and on the value-producing forms alike
#[test]
fn from_item_alias_adds_or_replaces() {
    let named = |alias: &str| {
        FromItem::Table(NamedTable {
            name: TableName::Table(Glyph::Table.into_name()),
            alias: Some(Name::runtime(alias)),
        })
    };

    assert_eq!(
        Glyph::Table.into_from_item().alias(Name::runtime("g")),
        named("g")
    );
    assert_eq!(
        Glyph::Table
            .into_from_item()
            .alias(Name::runtime("g"))
            .alias(Name::runtime("h")),
        named("h")
    );
    assert_eq!(
        (Name::runtime("schema"), Glyph::Table)
            .into_from_item()
            .alias(Name::runtime("g")),
        FromItem::Table(NamedTable {
            name: TableName::SchemaTable(Name::runtime("schema"), Glyph::Table.into_name()),
            alias: Some(Name::runtime("g")),
        })
    );
    assert_eq!(
        FromItem::ValuesList(vec![], Name::runtime("v")).alias(Name::runtime("w")),
        FromItem::ValuesList(vec![], Name::runtime("w"))
    );

    let fragment = || SqlTemplate::from_sql("SELECT 1", []).unwrap();
    assert_eq!(
        FromItem::Template(fragment(), Name::runtime("t")).alias(Name::runtime("u")),
        FromItem::Template(fragment(), Name::runtime("u"))
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    a column of a from item is qualified by its
// alias when it has one, otherwise by the table it names
#[test]
fn from_item_qualifier_prefers_the_alias() {
    let named = (Name::runtime("schema"), Glyph::Table).into_from_item();
    assert_eq!(named.qualifier().to_string(), "glyph");
    assert_eq!(
        named
            .clone()
            .alias(Name::runtime("g"))
            .qualifier()
            .to_string(),
        "g"
    );
    assert_eq!(
        named.table_name(),
        Some(&(Name::runtime("schema"), Glyph::Table).into_table_name())
    );

    let table = (Name::runtime("schema"), Glyph::Table).into_named_table();
    assert_eq!(table.qualifier().to_string(), "glyph");
    assert_eq!(table.alias(Name::runtime("g")).qualifier().to_string(), "g");

    let values = FromItem::ValuesList(vec![], Name::runtime("v"));
    assert_eq!(values.qualifier().to_string(), "v");
    assert_eq!(values.table_name(), None);

    let template = FromItem::Template(
        SqlTemplate::from_sql("SELECT 1", []).unwrap(),
        Name::runtime("t"),
    );
    assert_eq!(template.qualifier().to_string(), "t");
    assert_eq!(template.table_name(), None);
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    the named form renders as dotted, quoted parts
// with an optional alias
#[test]
fn named_from_item_forms_render() {
    let rendered =
        |from_item: FromItem| Query::select().column(Asterisk).from(from_item).to_string();

    assert_eq!(
        rendered(Glyph::Table.into_from_item()),
        r#"SELECT * FROM "glyph""#
    );
    assert_eq!(
        rendered((Name::runtime("schema"), Glyph::Table).into_from_item()),
        r#"SELECT * FROM "schema"."glyph""#
    );
    assert_eq!(
        rendered(Glyph::Table.into_from_item().alias(Name::runtime("g"))),
        r#"SELECT * FROM "glyph" AS "g""#
    );
    assert_eq!(
        rendered(
            (Name::runtime("schema"), Glyph::Table)
                .into_from_item()
                .alias(Name::runtime("g"))
        ),
        r#"SELECT * FROM "schema"."glyph" AS "g""#
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    the write statements take the same named table,
// and PostgreSQL accepts the alias each of them renders
// [spec:pgorm:def:sql.ast.insert+3/test]
// [spec:pgorm:req:sql.ast.update+5/test]
// [spec:pgorm:def:sql.ast.delete+4/test]
#[test]
fn aliased_dml_targets_render() {
    let target = || {
        (Name::runtime("schema"), Glyph::Table)
            .into_named_table()
            .alias(Name::runtime("g"))
    };

    assert_eq!(
        Query::insert()
            .into_table(target())
            .columns([Glyph::Image])
            .values_panic(["12A".into()])
            .to_string(),
        r#"INSERT INTO "schema"."glyph" AS "g" ("image") VALUES ('12A')"#
    );
    assert_eq!(
        Query::update()
            .table(target())
            .value(Glyph::Aspect, 1.23)
            .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).eq(1))
            .to_string(),
        r#"UPDATE "schema"."glyph" AS "g" SET "aspect" = 1.23 WHERE "g"."id" = 1"#
    );
    assert_eq!(
        Query::delete()
            .from_table(target())
            .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).eq(1))
            .to_string(),
        r#"DELETE FROM "schema"."glyph" AS "g" WHERE "g"."id" = 1"#
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    the three value-producing forms, all with a
// mandatory alias
#[test]
fn value_producing_from_item_forms_render() {
    let sub_query = FromItem::SubQuery(
        Query::select().column(Glyph::Id).from(Glyph::Table).take(),
        Name::runtime("sub"),
    );
    assert_eq!(
        Query::select().column(Asterisk).from(sub_query).to_string(),
        r#"SELECT * FROM (SELECT "id" FROM "glyph") AS "sub""#
    );

    let values_list = FromItem::ValuesList(
        vec![
            (1i32, "a").into_value_tuple(),
            (2i32, "b").into_value_tuple(),
        ],
        Name::runtime("v"),
    );
    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(values_list)
            .to_string(),
        r#"SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS "v""#
    );

    let function_call = FromItem::FunctionCall(
        Func::named(Name::runtime("generate_series")).arg(1i32),
        Name::runtime("f"),
    );
    assert_eq!(
        Query::select()
            .column(Asterisk)
            .from(function_call)
            .to_string(),
        r#"SELECT * FROM generate_series(1) AS "f""#
    );

    let template = FromItem::Template(
        SqlTemplate::from_sql(r#"SELECT "id" FROM "glyph""#, []).unwrap(),
        Name::runtime("t"),
    );
    assert_eq!(
        Query::select().column(Asterisk).from(template).to_string(),
        "SELECT * FROM (SELECT \"id\" FROM \"glyph\"\n) AS \"t\""
    );
}

// [spec:pgorm:def:sql.types.table-ref+7/test]    a fragment in relation position keeps its
// non-marker text verbatim and renumbers its markers into the enclosing statement's parameter
// space, so it composes with a statement that binds values of its own
// [spec:pgorm:req:sql.render.subquery+5/test]
#[test]
fn template_from_item_renumbers_into_the_enclosing_statement() {
    let fragment = SqlTemplate::from_sql(
        r#"SELECT "id" FROM "glyph" WHERE "aspect" > $1"#,
        [2i32.into()],
    )
    .unwrap();

    // The fragment's own `$1` lands after the value the enclosing statement
    // bound before it, and the statement's later value lands after that.
    let (sql, values) = Query::select()
        .column(Asterisk)
        .from_subquery(
            Query::select().expr(Expr::val(1i32)).take(),
            Name::runtime("before"),
        )
        .from(FromItem::Template(fragment, Name::runtime("g")))
        .and_where(Expr::col((Name::runtime("g"), Glyph::Id)).lt(9i32))
        .build();

    assert_eq!(
        sql,
        "SELECT * FROM (SELECT $1) AS \"before\", \
         (SELECT \"id\" FROM \"glyph\" WHERE \"aspect\" > $2\n) AS \"g\" \
         WHERE \"g\".\"id\" < $3"
    );
    assert_eq!(values.0, vec![1i32.into(), 2i32.into(), 9i32.into()]);
}

// [spec:pgorm:req:sql.render.custom-expr+3/test]    `from_sql` reads the `$` grammar of real SQL
// — quoted regions and comments opaque, `$$` a dollar-quote opener — and refuses a census the
// supplied values cannot satisfy
#[test]
fn template_from_sql_reads_real_sql() {
    let opaque = SqlTemplate::from_sql(
        "SELECT $1::int4 /* $99 */, $$ $99 $$, ' $99 ' -- $99\n",
        [7i32.into()],
    )
    .unwrap();

    let (sql, values) = Query::select()
        .column(Asterisk)
        .from(FromItem::Template(opaque, Name::runtime("t")))
        .build();

    assert_eq!(
        sql,
        "SELECT * FROM (SELECT $1::int4 /* $99 */, $$ $99 $$, ' $99 ' -- $99\n\n) AS \"t\""
    );
    assert_eq!(values.0, vec![7i32.into()]);

    // A marker with no value behind it, and a value nothing reads.
    assert_eq!(
        SqlTemplate::from_sql("SELECT $1, $2", [7i32.into()]),
        Err(Error::Template {
            template: "SELECT $1, $2".to_owned(),
            reason: TemplateError::IndexOutOfRange {
                index: 2,
                supplied: 1
            },
        })
    );
    assert_eq!(
        SqlTemplate::from_sql("SELECT $1", [7i32.into(), 8i32.into()]),
        Err(Error::Template {
            template: "SELECT $1".to_owned(),
            reason: TemplateError::UnreferencedValue {
                index: 2,
                supplied: 2
            },
        })
    );

    // `$$` is a dollar-quote opener here, not the `$` escape an authored
    // template spells it as, so the two constructors read the same text
    // differently: `from_sql` keeps the body, `new` collapses each `$$` to one
    // literal `$`.
    let rendered = |template| {
        Query::select()
            .column(Asterisk)
            .from(FromItem::Template(template, Name::runtime("t")))
            .to_string()
    };
    assert_eq_unparsed!(
        rendered(SqlTemplate::from_sql("SELECT $$ a $$", []).unwrap()),
        "SELECT * FROM (SELECT $$ a $$\n) AS \"t\""
    );
    assert_eq_unparsed!(
        rendered(SqlTemplate::new("SELECT $$ a $$", []).unwrap()),
        "SELECT * FROM (SELECT $ a $\n) AS \"t\""
    );
}

// [spec:pgorm:def:sql.types.opers+6/test]    `Not` is the only unary operator
#[test]
fn the_only_unary_operator_is_not() {
    let not_true = SimpleExpr::Unary(UnOper::Not, Box::new(SimpleExpr::from(true)));

    assert_eq!(
        Query::select().expr(not_true).to_string(),
        "SELECT NOT TRUE"
    );
}

// [spec:pgorm:def:sql.types.opers+6/test]    the whole binary operator vocabulary, including the
// `Custom` escape hatch
#[test]
fn the_binary_operator_vocabulary_is_complete() {
    let rendered = |op: BinOper| {
        Query::select()
            .expr(Expr::col(Glyph::Aspect).binary(op, Expr::val(1)))
            .to_string()
    };

    for (op, lexeme) in [
        (BinOper::And, "AND"),
        (BinOper::Or, "OR"),
        (BinOper::Like, "LIKE"),
        (BinOper::NotLike, "NOT LIKE"),
        (BinOper::ILike, "ILIKE"),
        (BinOper::NotILike, "NOT ILIKE"),
        (BinOper::Is, "IS"),
        (BinOper::IsNot, "IS NOT"),
        (BinOper::IsDistinctFrom, "IS DISTINCT FROM"),
        (BinOper::IsNotDistinctFrom, "IS NOT DISTINCT FROM"),
        (BinOper::In, "IN"),
        (BinOper::NotIn, "NOT IN"),
        (BinOper::Between, "BETWEEN"),
        (BinOper::NotBetween, "NOT BETWEEN"),
        (BinOper::BetweenSymmetric, "BETWEEN SYMMETRIC"),
        (BinOper::NotBetweenSymmetric, "NOT BETWEEN SYMMETRIC"),
        (BinOper::As, "AS"),
        (BinOper::Equal, "="),
        (BinOper::NotEqual, "<>"),
        (BinOper::SmallerThan, "<"),
        (BinOper::GreaterThan, ">"),
        (BinOper::SmallerThanOrEqual, "<="),
        (BinOper::GreaterThanOrEqual, ">="),
        (BinOper::Add, "+"),
        (BinOper::Sub, "-"),
        (BinOper::Mul, "*"),
        (BinOper::Div, "/"),
        (BinOper::Mod, "%"),
        (BinOper::LShift, "<<"),
        (BinOper::RShift, ">>"),
        (BinOper::Matches, "@@"),
        (BinOper::Contains, "@>"),
        (BinOper::Contained, "<@"),
        (BinOper::Concatenate, "||"),
        (BinOper::Overlap, "&&"),
        (BinOper::Similarity, "%"),
        (BinOper::WordSimilarity, "<%"),
        (BinOper::StrictWordSimilarity, "<<%"),
        (BinOper::SimilarityDistance, "<->"),
        (BinOper::WordSimilarityDistance, "<<->"),
        (BinOper::StrictWordSimilarityDistance, "<<<->"),
        (BinOper::GetJsonField, "->"),
        (BinOper::CastJsonField, "->>"),
        (BinOper::GetJsonPath, "#>"),
        (BinOper::CastJsonPath, "#>>"),
        (BinOper::HasJsonKey, "?"),
        (BinOper::HasAnyJsonKeys, "?|"),
        (BinOper::HasAllJsonKeys, "?&"),
        (BinOper::JsonPathExists, "@?"),
        (BinOper::Regex, "~"),
        (BinOper::RegexCaseInsensitive, "~*"),
        (BinOper::NotRegex, "!~"),
        (BinOper::NotRegexCaseInsensitive, "!~*"),
        (BinOper::NotExtendRightOf, "&<"),
        (BinOper::NotExtendLeftOf, "&>"),
        (BinOper::AdjacentTo, "-|-"),
        (BinOper::AtTimeZone, "AT TIME ZONE"),
        (BinOper::EuclideanDistance, "<->"),
        (BinOper::NegativeInnerProduct, "<#>"),
        (BinOper::CosineDistance, "<=>"),
        (BinOper::Raw("~~"), "~~"),
    ] {
        assert_eq_unparsed!(
            rendered(op),
            format!(r#"SELECT "aspect" {lexeme} 1"#),
            "unexpected rendering for {op:?}"
        );
    }
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]    the null-safe comparisons join the IS family,
// so they render bare under a logical operator rather than parenthesised
#[test]
fn null_safe_comparisons_bind_like_the_is_family() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).is_distinct_from(1))
            .and_where(Expr::col(Glyph::Image).is_not_distinct_from("a"))
            .to_string(),
        [
            r#"SELECT "id" FROM "glyph""#,
            r#"WHERE "aspect" IS DISTINCT FROM 1 AND "image" IS NOT DISTINCT FROM 'a'"#,
        ]
        .join(" ")
    );
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]    SYMMETRIC is part of the operator, so the
// ternary's `AND` still unwraps and the bounds render bare
#[test]
fn symmetric_between_keeps_its_bounds_unparenthesised() {
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).between_symmetric(10, 1))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" BETWEEN SYMMETRIC 10 AND 1"#
    );
    assert_eq!(
        Query::select()
            .column(Glyph::Id)
            .from(Glyph::Table)
            .and_where(Expr::col(Glyph::Aspect).not_between_symmetric(10, 1))
            .to_string(),
        r#"SELECT "id" FROM "glyph" WHERE "aspect" NOT BETWEEN SYMMETRIC 10 AND 1"#
    );
}

// [spec:pgorm:req:sql.ast.expr.operators+6/test]    AT TIME ZONE takes an ordinary expression on
// the right, so a bound zone name and a column both reach it
#[test]
fn at_time_zone_takes_any_zone_expression() {
    assert_eq!(
        Query::select()
            .expr(Expr::col(Glyph::Aspect).at_time_zone("UTC"))
            .from(Glyph::Table)
            .to_string(),
        r#"SELECT "aspect" AT TIME ZONE 'UTC' FROM "glyph""#
    );
    assert_eq!(
        Query::select()
            .expr(Expr::col(Glyph::Aspect).at_time_zone(Expr::col(Glyph::Image)))
            .from(Glyph::Table)
            .to_string(),
        r#"SELECT "aspect" AT TIME ZONE "image" FROM "glyph""#
    );
}

// [spec:pgorm:def:sql.types.column-type+11/test]    `StringLen` parameterises varchar and the
// convenience constructors go through it
#[test]
fn string_len_and_the_convenience_constructors() {
    assert_eq!(StringLen::default(), StringLen::None);
    assert_eq!(
        ColumnType::string(Some(64)),
        ColumnType::String(StringLen::N(64))
    );
    assert_eq!(
        ColumnType::string(None),
        ColumnType::String(StringLen::None)
    );
    assert_eq!(
        ColumnType::named("citext"),
        ColumnType::Named(TypeName::new(Name::runtime("citext")))
    );
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    only the integer trio has a serial spelling
#[test]
fn serial_spelling_covers_the_integer_trio() {
    assert_eq!(
        ColumnType::SmallInteger.serial_spelling(),
        Some("smallserial")
    );
    assert_eq!(ColumnType::Integer.serial_spelling(), Some("serial"));
    assert_eq!(ColumnType::BigInteger.serial_spelling(), Some("bigserial"));

    for other in [ColumnType::Uuid, ColumnType::Text, ColumnType::Bytea] {
        assert_eq!(other.serial_spelling(), None);
    }
}

// [spec:pgorm:req:sql.ddl.column-def+13/test]    a type with no serial form renders itself
#[test]
fn auto_increment_without_serial_form_renders_type() {
    assert_eq!(
        Table::create(Glyph::Table)
            .col(ColumnDef::new(Glyph::Id).uuid().auto_increment())
            .to_string(),
        r#"CREATE TABLE "glyph" ( "id" uuid )"#
    );
}

// [spec:pgorm:def:sql.types.column-type+11/test]    equality compares parameters, renders
// `Custom`/`Enum` identifiers, recurses into `Array`, compares a range's range type, and
// otherwise compares discriminants
#[test]
fn column_type_equality_semantics() {
    // Parameterised variants compare their parameters.
    assert_eq!(ColumnType::Char(Some(3)), ColumnType::Char(Some(3)));
    assert_ne!(ColumnType::Char(Some(3)), ColumnType::Char(Some(4)));
    assert_eq!(
        ColumnType::Decimal(Some((10, 2))),
        ColumnType::Decimal(Some((10, 2)))
    );
    assert_ne!(
        ColumnType::Decimal(Some((10, 2))),
        ColumnType::Decimal(None)
    );
    assert_eq!(
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::Second(Some(
            IntervalPrecision::P3
        )))),
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::Second(Some(
            IntervalPrecision::P3
        ))))
    );
    assert_ne!(
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::Hour)),
        ColumnType::Interval(IntervalSpec::Fields(PgInterval::Day))
    );
    assert_ne!(
        ColumnType::Interval(IntervalSpec::Any(None)),
        ColumnType::Interval(IntervalSpec::Any(Some(IntervalPrecision::P0)))
    );

    // `Custom` compares by rendered identifier, not by concrete iden type.
    assert_eq!(
        ColumnType::named("aspect"),
        ColumnType::Named(TypeName::new(Glyph::Aspect))
    );
    assert_ne!(ColumnType::named("aspect"), ColumnType::named("image"));

    // `Enum` compares name and variant list, both by rendered text.
    let tea = ColumnType::Enum {
        schema: None,
        name: Name::runtime("tea"),
        variants: vec![Name::runtime("green"), Name::runtime("black")],
    };
    let same_tea = ColumnType::Enum {
        schema: None,
        name: Name::runtime("tea"),
        variants: vec![Name::runtime("green"), Name::runtime("black")],
    };
    let other_tea = ColumnType::Enum {
        schema: None,
        name: Name::runtime("tea"),
        variants: vec![Name::runtime("green")],
    };
    assert_eq!(tea, same_tea);
    assert_ne!(tea, other_tea);

    // `Array` recurses into its element type.
    assert_eq!(
        ColumnType::Array(Arc::new(ColumnType::Integer)),
        ColumnType::Array(Arc::new(ColumnType::Integer))
    );
    assert_ne!(
        ColumnType::Array(Arc::new(ColumnType::Integer)),
        ColumnType::Array(Arc::new(ColumnType::Text))
    );

    // `Range` and `Multirange` compare the range type they name.
    assert_eq!(
        ColumnType::Range(RangeType::Date),
        ColumnType::Range(RangeType::Date)
    );
    assert_ne!(
        ColumnType::Range(RangeType::Date),
        ColumnType::Range(RangeType::Timestamp)
    );
    assert_ne!(
        ColumnType::Multirange(RangeType::Int4),
        ColumnType::Multirange(RangeType::Int8)
    );
    assert_ne!(
        ColumnType::Range(RangeType::Int4),
        ColumnType::Multirange(RangeType::Int4)
    );

    // Everything else compares discriminants.
    assert_eq!(ColumnType::Text, ColumnType::Text);
    assert_ne!(ColumnType::Text, ColumnType::Json);
    assert_ne!(ColumnType::Cidr, ColumnType::Inet);
    assert_ne!(ColumnType::MacAddr, ColumnType::LTree);
}

// [spec:pgorm:def:sql.types.column-type+11/test]    `PgInterval` displays as SQL keywords and
// has a case-insensitive `TryFrom<&str>` inverse
#[test]
fn pg_interval_display_and_parse_round_trip() {
    let all = [
        (PgInterval::Year, "YEAR"),
        (PgInterval::Month, "MONTH"),
        (PgInterval::Day, "DAY"),
        (PgInterval::Hour, "HOUR"),
        (PgInterval::Minute, "MINUTE"),
        (PgInterval::Second(None), "SECOND"),
        (PgInterval::YearToMonth, "YEAR TO MONTH"),
        (PgInterval::DayToHour, "DAY TO HOUR"),
        (PgInterval::DayToMinute, "DAY TO MINUTE"),
        (PgInterval::DayToSecond(None), "DAY TO SECOND"),
        (PgInterval::HourToMinute, "HOUR TO MINUTE"),
        (PgInterval::HourToSecond(None), "HOUR TO SECOND"),
        (PgInterval::MinuteToSecond(None), "MINUTE TO SECOND"),
    ];

    for (field, keywords) in all {
        assert_eq!(field.to_string(), keywords);
        assert_eq!(PgInterval::try_from(keywords).unwrap(), field);
    }

    // A precision is spelled by the field that carries it.
    assert_eq!(
        PgInterval::MinuteToSecond(Some(IntervalPrecision::P6)).to_string(),
        "MINUTE TO SECOND(6)"
    );

    // Case and surrounding whitespace are forgiven; anything else is an error.
    assert_eq!(
        PgInterval::try_from("  year to month ").unwrap(),
        PgInterval::YearToMonth
    );
    assert_eq!(
        PgInterval::try_from("century".to_owned()).unwrap_err(),
        "Cannot turn \"CENTURY\" into a Postgres interval field".to_owned()
    );
}

// [spec:pgorm:def:sql.types.column-type+11/test]    the precision vocabulary is the closed set
// PostgreSQL accepts, and nothing outside it constructs
#[test]
fn interval_precision_is_zero_through_six() {
    for digits in 0..=6u8 {
        let precision = IntervalPrecision::new(digits).expect("0..=6 are precisions");
        assert_eq!(precision.digits(), digits);
        assert_eq!(precision.to_string(), digits.to_string());
    }

    assert_eq!(IntervalPrecision::new(7), None);
    assert_eq!(IntervalPrecision::new(u8::MAX), None);
}
